//! `Layout/LineContinuationLeadingSpace` — in a string broken over lines by a
//! backslash, the blanks at a break belong at the end of the earlier part.
//!
//! A port of RuboCop's `on_dstr` (97f30c4, MIT) at the fixed
//! `EnforcedStyle: trailing`. Upstream walks consecutive raw source lines of a
//! parser `dstr`: a line ending in `\` + `\n` is a continuation unless a
//! multi-line child of the `dstr` covers it (the backslash then belongs to a
//! token spanning to the next line), and the next line is reported when it
//! matches `/\A\s*['"]\s+/`: the blanks right after its opening quote.
//!
//! Of the Prism nodes the pinned translator turns into a `dstr`, only an
//! implicit concatenation of adjacent string literals (an
//! `InterpolatedStringNode` without an opening) can have such a line: in a
//! delimited string or a multi-line literal every line but the last ends
//! inside a multi-line child (a string piece, which the translator splits
//! at newlines, or an interpolation), and a heredoc's `dstr` is its one-line
//! opener. The children are the concatenated parts. As upstream, a backslash
//! before `\r\n` is not a continuation.
//!
//! The correction moves the blanks before the closing quote of the earlier
//! part, one contiguous edit from that quote to the end of the blanks. The
//! concatenated value is unchanged: the bytes moved are literal blanks of the
//! later part's text, inserted after the complete text of the earlier one,
//! and no escape, interpolation or delimiter is touched. It is withheld unless
//! the quote before the continuation closes one part and the quote opening the
//! next line opens the following part, both quoted literals (upstream's
//! `LINE_1_ENDING` match fails and RuboCop raises an error when the earlier
//! part ends with another delimiter, as `%q(a) \`), when the blanks reach the
//! end of the line (they would move a newline), and for the listed
//! source observers.
//!
//! Upstream's `autocorrect_incompatible_with` (`Style/StringLiterals`) keeps
//! RuboCop's iterative rounds from applying both cops' edits from the same
//! original source, where StringLiterals' re-quoted copy of a part would
//! restore the moved blanks. It is not declared: rustocop withholds a cop's
//! edits whenever a declared companion is selected, and the default-on
//! StringLiterals would disable every edit of this cop. A StringLiterals edit
//! replaces a whole part, which overlaps this cop's edit of the same two
//! parts, so one `--fix` pass applies only one of them (`corrections::apply`),
//! and the next pass applies the other to the updated source.
use std::ops::Range;

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct LineContinuationLeadingSpace;

const MSG: &str = "Move leading spaces to the end of the previous line.";

/// Ruby's regexp `\s`.
fn blank(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
}

fn quote(byte: u8) -> bool {
    matches!(byte, b'\'' | b'"')
}

/// `TRAILING_STYLE_OFFENSE` on a raw line: the range (relative to the line
/// start) of the blanks after the opening quote.
fn leading_spaces(line: &[u8]) -> Option<Range<usize>> {
    let indent = line.iter().take_while(|b| blank(**b)).count();
    if !quote(*line.get(indent)?) {
        return None;
    }
    let start = indent + 1;
    let blanks = line[start..].iter().take_while(|b| blank(**b)).count();
    (blanks > 0).then_some(start..start + blanks)
}

/// The offset of `LINE_1_ENDING`'s quote: the quote before the blanks before
/// the line's final `\` + `\n`.
fn closing_quote(file: &SourceFile, line_start: usize, next_line_start: usize) -> Option<usize> {
    let line = &file.bytes[line_start..next_line_start];
    let before = line.len().checked_sub(2)?;
    let blanks = line[..before]
        .iter()
        .rev()
        .take_while(|b| blank(**b))
        .count();
    let at = before.checked_sub(blanks + 1)?;
    quote(line[at]).then_some(line_start + at)
}

/// A part's line span, as parser's `first_line`/`last_line`.
fn lines(file: &SourceFile, part: &Node<'_>) -> (u32, u32) {
    let location = part.location();
    (
        file.line_of(location.start_offset()),
        file.last_line_of(location),
    )
}

/// A quoted string part's opening and closing delimiters.
fn delimiters(part: &Node<'_>) -> Option<(Range<usize>, Range<usize>)> {
    let (opening, closing) = match part {
        Node::StringNode { .. } => {
            let string = part.as_string_node()?;
            (string.opening_loc()?, string.closing_loc()?)
        }
        Node::InterpolatedStringNode { .. } => {
            let string = part.as_interpolated_string_node()?;
            (string.opening_loc()?, string.closing_loc()?)
        }
        _ => return None,
    };
    let span = |l: ruby_prism::Location<'_>| l.start_offset()..l.end_offset();
    Some((span(opening), span(closing)))
}

impl Cop for LineContinuationLeadingSpace {
    fn name(&self) -> &'static str {
        "Layout/LineContinuationLeadingSpace"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_interpolated_string_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(dstr) = node.as_interpolated_string_node() else {
            return;
        };
        let location = dstr.location();
        if dstr.opening_loc().is_some() || !location.as_slice().contains(&b'\\') {
            return;
        }
        let parts: Vec<Node<'pr>> = dstr.parts().iter().collect();
        let first = file.line_of(location.start_offset());
        let last = file.last_line_of(location);
        // Lines a multi-line child covers (`first_line...last_line`).
        let mut covered = vec![false; (last - first) as usize];
        for part in &parts {
            let (from, to) = lines(file, part);
            for line in from..to {
                covered[(line - first) as usize] = true;
            }
        }
        for line in first..last {
            let one = file.line_start(line);
            let two = file.line_start(line + 1);
            let three = file.line_start(line + 2);
            if !file.bytes[one..two].ends_with(b"\\\n") || covered[(line - first) as usize] {
                continue;
            }
            let Some(blanks) = leading_spaces(&file.bytes[two..three]) else {
                continue;
            };
            let blanks = two + blanks.start..two + blanks.end;
            let (line_no, col) = file.line_col(blanks.start);
            let correction = cx
                .corrections_requested()
                .then(|| correction(file, cx, &parts, one, two, &blanks))
                .flatten();
            out.push(Offense {
                correction,
                cop: self.name(),
                severity: self.severity(),
                message: MSG.into(),
                line: line_no,
                end_line: file.line_of(blanks.end - 1),
                col,
            });
        }
    }
}

fn correction(
    file: &SourceFile,
    cx: &NodeContext<'_, '_>,
    parts: &[Node<'_>],
    one: usize,
    two: usize,
    blanks: &Range<usize>,
) -> Option<Correction> {
    let closing = closing_quote(file, one, two)?;
    let opening = blanks.start - 1;
    // The quotes must be the closing and opening delimiters of two
    // consecutive parts (in source order).
    let next = parts.partition_point(|p| p.location().start_offset() <= opening);
    let consecutive = next >= 2 && {
        let (Some((_, closes)), Some((opens, _))) =
            (delimiters(&parts[next - 2]), delimiters(&parts[next - 1]))
        else {
            return None;
        };
        closes == (closing..closing + 1) && opens == (opening..opening + 1)
    };
    if !consecutive || file.bytes[blanks.clone()].contains(&b'\n') {
        return None;
    }
    let root = cx.ancestors().last()?;
    if file
        .class_method_bindings(root)
        .has_listed_source_observer(file.data_start().is_some())
    {
        return None;
    }
    let mut replacement = file.bytes[blanks.clone()].to_vec();
    replacement.extend_from_slice(&file.bytes[closing..blanks.start]);
    Some(Correction {
        range: closing..blanks.end,
        replacement: String::from_utf8(replacement).ok()?,
    })
}

#[cfg(test)]
mod tests;
