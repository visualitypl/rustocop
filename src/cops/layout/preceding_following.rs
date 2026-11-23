//! RuboCop's `PrecedingFollowingAlignment` (97f30c4, MIT) over the
//! parser-gem token projection: whether a token or argument is aligned with
//! something on the nearest relevant line above or below. Built once per file
//! (cached on `SourceFile`) with linear line indexes, so each query is O(1)
//! plus the compared text.
use std::ops::Range;

use crate::source::{SourceFile, tokens::assignment_or_comparison};

/// Per-line facts of `processed_source.lines`.
pub(crate) struct Lines {
    /// Byte offsets of each character start plus the line end, line after
    /// line; [`Lines::chars`] reads one line's.
    offsets: Vec<usize>,
    /// Where each line's entries start in `offsets`, plus the total length.
    bounds: Vec<usize>,
    /// Character index of the first non-`\s` character (`line =~ /\S/`).
    first: Vec<Option<usize>>,
    /// Lines whose comment begins the line (`aligned_comment_lines`).
    comment_lines: Vec<bool>,
    /// Nearest relevant line above/below, ignoring indentation.
    nearest: Vec<[Option<usize>; 2]>,
    /// Nearest line above/below at the same indentation, not past a less
    /// indented line.
    same_indent: Vec<[Option<usize>; 2]>,
    /// The first assignment or comparison token within each line.
    operator: Vec<Option<usize>>,
}

/// Ruby's `\s`.
pub(crate) fn space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
}

/// The number of `processed_source.lines`: the buffer's source lines (a
/// trailing newline starts one more, empty line), stopping at `__END__`.
pub(crate) fn line_count(file: &SourceFile) -> usize {
    match file.data_start() {
        Some(data) => file.line_of(data) as usize - 1,
        None => file.bytes.iter().filter(|&&b| b == b'\n').count() + 1,
    }
}

impl Lines {
    pub(crate) fn build(file: &SourceFile) -> Self {
        let count = line_count(file);
        let mut offsets = Vec::with_capacity(file.bytes.len() + count);
        let mut bounds = Vec::with_capacity(count + 1);
        for line in 1..=count {
            let mut start = file.line_start(line as u32);
            let end = start + file.line_text(line as u32).len();
            if line == 1 && file.bytes.starts_with(b"\xef\xbb\xbf") {
                start = (start + 3).min(end);
            }
            bounds.push(offsets.len());
            offsets.extend(file.character_offsets(start..end));
            offsets.push(end);
        }
        bounds.push(offsets.len());
        let chars = |i: usize| &offsets[bounds[i]..bounds[i + 1]];
        let first: Vec<Option<usize>> = (0..count)
            .map(|i| {
                let offsets = chars(i);
                offsets[..offsets.len() - 1]
                    .iter()
                    .position(|&offset| !space(file.bytes[offset]))
            })
            .collect();
        let mut comment_lines = vec![false; count];
        for comment in file.comments() {
            let (line, column) = file.line_col(comment.start);
            let index = line as usize - 1;
            if index < count && first[index] == Some(column as usize - 1) {
                comment_lines[index] = true;
            }
        }
        let relevant = |i: usize| first[i].is_some() && !comment_lines[i];
        let mut nearest = vec![[None; 2]; count];
        let mut same_indent = vec![[None; 2]; count];
        for direction in 0..2 {
            let mut previous = None;
            let mut stack: Vec<(usize, usize)> = Vec::new();
            for n in 0..count {
                let i = if direction == 0 { n } else { count - 1 - n };
                nearest[i][direction] = previous;
                let Some(indent) = first[i] else {
                    continue;
                };
                if !relevant(i) {
                    // A comment line is skipped by the scan: answer without
                    // changing the stack (rare: only ranges on such lines).
                    same_indent[i][direction] = stack
                        .iter()
                        .rev()
                        .find(|(level, _)| *level <= indent)
                        .filter(|(level, _)| *level == indent)
                        .map(|(_, line)| *line);
                    continue;
                }
                while stack.last().is_some_and(|(level, _)| *level > indent) {
                    stack.pop();
                }
                if stack.last().is_some_and(|(level, _)| *level == indent) {
                    same_indent[i][direction] = stack.pop().map(|(_, line)| line);
                }
                stack.push((indent, i));
                previous = Some(i);
            }
        }
        // `tokens_within(line_range)`: sorted tokens from the first starting
        // in the line through the first ending at or after its end.
        let tokens = file.parser_tokens();
        let mut sorted: Vec<usize> = (0..tokens.len()).collect();
        sorted.sort_by_key(|&i| tokens[i].start);
        let operator = (0..count)
            .map(|i| {
                let offsets = chars(i);
                let (start, end) = (file.line_start(i as u32 + 1), *offsets.last().unwrap());
                let from = sorted.partition_point(|&t| tokens[t].start < start);
                let to = sorted
                    .partition_point(|&t| tokens[t].end < end)
                    .min(sorted.len().saturating_sub(1));
                // A token spanning the whole line leaves Ruby's `a[from..to]`
                // empty.
                sorted
                    .get(from..=to)
                    .unwrap_or_default()
                    .iter()
                    .copied()
                    .find(|&t| assignment_or_comparison(tokens[t].kind))
            })
            .collect();
        Self {
            offsets,
            bounds,
            first,
            comment_lines,
            nearest,
            same_indent,
            operator,
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.bounds.len() - 1
    }

    /// Byte offsets of each character start plus the end of 0-based line `i`.
    fn chars(&self, i: usize) -> &[usize] {
        &self.offsets[self.bounds[i]..self.bounds[i + 1]]
    }

    /// The text of 1-based `line`'s character `column` onwards, if it exists.
    fn text_at_column<'f>(
        &self,
        file: &'f SourceFile,
        line: usize,
        column: usize,
    ) -> Option<&'f [u8]> {
        let offsets = (line < self.count()).then(|| self.chars(line))?;
        let start = *offsets.get(column)?;
        Some(&file.bytes[start..*offsets.last().unwrap()])
    }

    /// `line_indentation`: leading `\s` characters of a 1-based line.
    pub(crate) fn indentation(&self, line: u32) -> usize {
        let i = line as usize - 1;
        self.first[i].unwrap_or(self.chars(i).len() - 1)
    }

    /// `String#blank?` of a 1-based line.
    pub(crate) fn blank(&self, line: u32) -> bool {
        self.first[line as usize - 1].is_none()
    }

    /// Whether 1-based `line` is in `aligned_comment_lines`.
    pub(crate) fn comment_line(&self, line: u32) -> bool {
        self.comment_lines
            .get(line as usize - 1)
            .copied()
            .unwrap_or(false)
    }

    /// `aligned_with_something?(range)`: the token or node text at `range`
    /// starting at 1-based `line`, 0-based character `column`, ending at
    /// character column `end_column`.
    pub(crate) fn aligned_with_something(
        &self,
        file: &SourceFile,
        range: Range<usize>,
        line: u32,
        column: usize,
        end_column: usize,
    ) -> bool {
        let i = line as usize - 1;
        if i >= self.count() {
            return false;
        }
        let check = |other: Option<usize>| {
            other.is_some_and(|other| self.aligned_token(file, &range, column, end_column, other))
        };
        let [above, below] = self.nearest[i];
        if check(above) || check(below) {
            return true;
        }
        let [above, below] = self.same_indent_of(i);
        check(above) || check(below)
    }

    fn same_indent_of(&self, i: usize) -> [Option<usize>; 2] {
        if self.first[i].is_some() {
            return self.same_indent[i];
        }
        // A blank base line has nil indentation: `index < nil` raises
        // upstream; no token or argument starts on such a line.
        [None, None]
    }

    /// `aligned_token?`: `aligned_words?` or `aligned_equals_operator?`.
    fn aligned_token(
        &self,
        file: &SourceFile,
        range: &Range<usize>,
        column: usize,
        end_column: usize,
        other: usize,
    ) -> bool {
        let text = &file.bytes[range.clone()];
        let chars = self.chars(other);
        let words = column > 0
            && chars.len() > column + 1
            && space(file.bytes[chars[column - 1]])
            && !space(file.bytes[chars[column]]);
        words
            || self
                .text_at_column(file, other, column)
                .is_some_and(|rest| rest.starts_with(text))
            || self.aligned_equals_operator(file, text, end_column, other)
    }

    fn aligned_equals_operator(
        &self,
        file: &SourceFile,
        text: &[u8],
        end_column: usize,
        other: usize,
    ) -> bool {
        let Some(token) = self.operator[other].map(|t| &file.parser_tokens()[t]) else {
            return false;
        };
        let aligned = end_column == token.end_column as usize;
        let equals = text.ends_with(b"=");
        aligned
            && (equals
                || (text == b"<<" && token.is_equal_sign())
                || (equals && token.kind == "tLSHFT"))
    }
}

pub(crate) static PRECEDING_FOLLOWING: crate::source::Fact<Lines> = crate::source::Fact::new();

impl crate::source::SourceFile {
    /// Line facts of RuboCop's PrecedingFollowingAlignment, shared by the
    /// spacing cops that allow alignment.
    pub(crate) fn preceding_following(&self) -> &Lines {
        PRECEDING_FOLLOWING.get(self, || Lines::build(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aligned(source: &str, needle: &str, line: u32) -> bool {
        let file = SourceFile::new("t.rb".into(), source.as_bytes().to_vec());
        let lines = Lines::build(&file);
        let start = file.line_start(line)
            + std::str::from_utf8(file.line_text(line))
                .unwrap()
                .find(needle)
                .unwrap();
        let end = start + needle.len();
        let column = file.line_col(start).1 as usize - 1;
        let end_column = file.line_col(end).1 as usize - 1;
        lines.aligned_with_something(&file, start..end, line, column, end_column)
    }

    #[test]
    fn words_identical_text_and_equals_operators_align() {
        assert!(aligned("a   = 1\nbcd = 2\n", "=", 1));
        assert!(aligned("x  foo\ny  foo\n", "foo", 2));
        assert!(aligned("ab = 1\nx +=  2\n", "+=", 2));
        assert!(!aligned("website(\"x\")\nname   = 1\n", "=", 2));
        // Comment lines are skipped, blank lines are not crossed for the
        // nearest line but the same-indentation pass skips deeper lines.
        assert!(aligned("a  = 1\n# c\nbb = 2\n", "=", 3));
        assert!(aligned("x  = {\n       a: 1\n     }\ny  = 2\n", "= 2", 4));
        assert!(!aligned(
            "if a\n  x  = 1\nend\nif b\n  y  = 1\nend\n",
            "=",
            5
        ));
    }

    #[test]
    fn line_count_and_indentation_follow_processed_source() {
        let file = SourceFile::new("t.rb".into(), b"a\n  b\n\n__END__\nx\n".to_vec());
        assert_eq!(line_count(&file), 3);
        let lines = Lines::build(&file);
        assert_eq!(lines.indentation(2), 2);
        assert!(lines.blank(3));
        let file = SourceFile::new("t.rb".into(), b"a\n".to_vec());
        assert_eq!(line_count(&file), 2);
    }
}
