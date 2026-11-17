//! `Layout/SpaceInsideArrayPercentLiteral` — one space between the items of
//! a `%i`/`%I`/`%w`/`%W` array literal.
//!
//! A port of RuboCop's cop (97f30c4, MIT). `on_array` through the
//! `PercentLiteral` mixin: a Prism `ArrayNode` whose opening is `%i`, `%I`,
//! `%w` or `%W` plus its delimiter (`[...]` arrays and `%q`/`%r`/`%s`/`%x`
//! literals are not checked). Upstream scans the text between the
//! delimiters with `/(?:[\S&&[^\\]](?:\\ )*)( {2,})(?=\S)/`: a non-blank
//! character other than `\`, any escaped spaces after it, then two or more
//! spaces before a non-blank character. Each run of spaces is reported
//! (`Use only a single space inside array percent literal.`). The scan is
//! ported per source character, so the second byte of a legacy double-byte
//! character (`0x5C` in Shift_JIS) is not read as a backslash; it is
//! textual, so an escaped backslash before the run (`a\\  b`) is not
//! matched, as upstream, and runs inside an interpolation are reported.
//! Multi-line literals are checked: spaces after a line break follow no
//! matching character.
//!
//! The correction replaces the run with one space, as upstream, when the run
//! lies between two of Prism's elements: it is then a separator, and the
//! literal's elements, their values and the encoding are unchanged (one
//! ASCII space for two or more). A run inside an element, which can only be
//! interpolated code (`%W(#{"a  b"})`, where collapsing it would change a
//! string), keeps its finding without an edit. The listed source
//! observers hold every edit.
use std::ops::Range;

use ruby_prism::Node;

use super::space_after_colon::ruby_space;
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceInsideArrayPercentLiteral;

const MSG: &str = "Use only a single space inside array percent literal.";

/// A percent literal of one of the requested types (`PercentLiteral#process`).
pub(super) struct PercentLiteral {
    /// Between the delimiters (`RangeHelp#contents_range`).
    pub contents: Range<usize>,
    /// The array's elements, in source order; empty for `%x`.
    pub elements: Vec<Range<usize>>,
    pub single_line: bool,
}

/// The percent literal `node` is, when its type (the opening without its
/// delimiter) is one of `types`.
pub(super) fn percent_literal(
    node: &Node<'_>,
    file: &SourceFile,
    types: &[&[u8]],
) -> Option<PercentLiteral> {
    let (opening, closing, elements) = if let Some(array) = node.as_array_node() {
        let elements = array
            .elements()
            .iter()
            .map(|e| e.location().start_offset()..e.location().end_offset())
            .collect();
        (array.opening_loc()?, array.closing_loc()?, elements)
    } else if let Some(command) = node.as_x_string_node() {
        (command.opening_loc(), command.closing_loc(), Vec::new())
    } else {
        let command = node.as_interpolated_x_string_node()?;
        (command.opening_loc(), command.closing_loc(), Vec::new())
    };
    let open = opening.as_slice();
    if !open.starts_with(b"%") || !types.contains(&&open[..open.len() - 1]) {
        return None;
    }
    let location = node.location();
    Some(PercentLiteral {
        contents: opening.end_offset()..closing.start_offset(),
        elements,
        single_line: file.line_of(location.start_offset()) == file.last_line_of(location),
    })
}

/// One source character between the delimiters, classified for upstream's
/// regexps (which match characters, not bytes).
#[derive(Clone, Copy)]
pub(super) struct Character {
    pub start: usize,
    /// The ASCII byte, `None` for a multi-byte character.
    pub ascii: Option<u8>,
}

impl Character {
    pub fn space(self) -> bool {
        self.ascii == Some(b' ')
    }

    pub fn backslash(self) -> bool {
        self.ascii == Some(b'\\')
    }

    /// Ruby regexp `\S`.
    pub fn non_blank(self) -> bool {
        self.ascii.is_none_or(|b| !ruby_space(b))
    }
}

/// The characters of `range` in the source encoding. A UTF-8 or ASCII span
/// is read byte by byte: no byte of a multi-byte UTF-8 character is ASCII.
pub(super) fn characters(file: &SourceFile, range: Range<usize>) -> Vec<Character> {
    let bytes = &file.bytes[range.clone()];
    if file.is_utf8() || bytes.is_ascii() {
        return bytes
            .iter()
            .enumerate()
            .map(|(i, &b)| Character {
                start: range.start + i,
                ascii: Some(b),
            })
            .collect();
    }
    let starts: Vec<usize> = file.character_offsets(range.clone()).collect();
    starts
        .iter()
        .enumerate()
        .map(|(i, &start)| {
            let end = starts.get(i + 1).copied().unwrap_or(range.end);
            Character {
                start,
                ascii: (end == start + 1 && file.bytes[start].is_ascii())
                    .then(|| file.bytes[start]),
            }
        })
        .collect()
}

/// The byte ranges of `MULTIPLE_SPACES_BETWEEN_ITEMS_REGEX`'s first group.
fn unnecessary_spaces(chars: &[Character], end: usize) -> Vec<Range<usize>> {
    let start_of = |i: usize| chars.get(i).map_or(end, |c| c.start);
    let mut runs = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].non_blank() || chars[i].backslash() {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j + 1 < chars.len() && chars[j].backslash() && chars[j + 1].space() {
            j += 2;
        }
        let mut k = j;
        while k < chars.len() && chars[k].space() {
            k += 1;
        }
        if k - j >= 2 && k < chars.len() && chars[k].non_blank() {
            runs.push(start_of(j)..start_of(k));
            i = k;
        } else {
            i += 1;
        }
    }
    runs
}

/// Whether `run` lies in the gap between two consecutive elements.
fn separator(elements: &[Range<usize>], run: &Range<usize>) -> bool {
    let before = elements.partition_point(|e| e.end <= run.start);
    before >= 1 && elements.get(before).is_some_and(|e| e.start >= run.end)
}

impl Cop for SpaceInsideArrayPercentLiteral {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideArrayPercentLiteral"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(literal) = percent_literal(node, file, &[b"%i", b"%I", b"%w", b"%W"]) else {
            return;
        };
        let chars = characters(file, literal.contents.clone());
        let runs = unnecessary_spaces(&chars, literal.contents.end);
        if runs.is_empty() {
            return;
        }
        let edit = cx.corrections_requested();
        for run in runs {
            let (line, col) = file.line_col(run.start);
            let correction = (edit && separator(&literal.elements, &run)).then(|| Correction {
                range: run,
                replacement: " ".into(),
            });
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
}

#[cfg(test)]
mod tests;
