//! `Layout/EndOfLine` — line endings follow the host platform.
//!
//! Fixed `EnforcedStyle: native`, which is `lf` on every host rustocop
//! supports (`Platform.windows?` is false): the first raw line, split after
//! each LF as Ruby's `each_line` does, that ends in CR or CR LF is reported
//! at its first column, and only that one. Lines after the last token are
//! not checked; without any token every raw line is, data section included.
//!
//! The last token follows the Prism token stream that RuboCop translates for
//! the parser gem: a heredoc body is lexed at its opener, before the rest of
//! that line and its newline token; `#` comments directly after the last
//! code line (no blank line between them) are followed by that line's
//! deferred newline token, which is then the last token; a `=begin` block
//! comment is one token at its first line; `__END__` and the data section
//! are not tokens.
//!
//! Detection only, as upstream: there is no correction, and `--fix` never
//! normalizes line endings.
use super::space_before_comment::{blank, bom_end};
use crate::{
    cops::Cop,
    offense::{Offense, Severity},
    source::SourceFile,
};

pub struct EndOfLine;

impl Cop for EndOfLine {
    fn name(&self) -> &'static str {
        "Layout/EndOfLine"
    }

    fn check_parse(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let last = last_token_line(result, file).unwrap_or(u32::MAX);
        let mut line = 0;
        for text in file.bytes.split_inclusive(|&b| b == b'\n') {
            line += 1;
            if line > last {
                return;
            }
            if text.ends_with(b"\r") || text.ends_with(b"\r\n") {
                out.push(Offense {
                    correction: None,
                    cop: self.name(),
                    severity: Severity::Convention,
                    message: "Carriage return character detected.".into(),
                    line,
                    end_line: line,
                    col: 1,
                });
                return;
            }
        }
    }
}

/// The line of upstream's `processed_source.tokens.last`; `None` when the
/// file has no token.
fn last_token_line(result: &ruby_prism::ParseResult<'_>, file: &SourceFile) -> Option<u32> {
    let bytes = &file.bytes;
    let end = file.data_start().unwrap_or(bytes.len());
    let comments = file.comments();
    let root = result.node();
    let heredocs = file.heredocs(&root);
    // Bytes that are not code tokens: comments and heredoc bodies with their
    // terminators (lexed at the opener).
    let mut skipped: Vec<std::ops::Range<usize>> = comments
        .iter()
        .cloned()
        .chain(heredocs.iter().map(|h| h.body.start..h.closing.end))
        .collect();
    skipped.sort_by_key(|range| range.start);
    let mut merged: Vec<std::ops::Range<usize>> = Vec::with_capacity(skipped.len());
    for range in skipped {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    let start = bom_end(file);
    let mut at = end;
    let mut code = None;
    while at > start {
        let byte = bytes[at - 1];
        let index = merged.partition_point(|range| range.start < at);
        if let Some(range) = index
            .checked_sub(1)
            .map(|i| &merged[i])
            .filter(|range| at <= range.end)
        {
            at = range.start;
        } else if blank(byte) || (byte == b'\\' && matches!(bytes.get(at), Some(b'\n' | b'\r'))) {
            at -= 1;
        } else {
            code = Some(at - 1);
            break;
        }
    }
    let after = match code {
        Some(code) => &comments[comments.partition_point(|c| c.start <= code)..],
        None => comments,
    };
    let code_line = code.map(|offset| file.line_of(offset));
    let Some(last) = after.last() else {
        return code_line;
    };
    let last_line = file.line_of(last.start);
    let (Some(code), Some(code_line)) = (code, code_line) else {
        return Some(last_line);
    };
    // The trailing run of `#` comments on consecutive lines.
    let hash = |c: &std::ops::Range<usize>| bytes[c.start] == b'#';
    let mut run = after.len() - 1;
    while run > 0
        && hash(&after[run])
        && hash(&after[run - 1])
        && file.line_of(after[run - 1].start) + 1 == file.line_of(after[run].start)
    {
        run -= 1;
    }
    if run != 0 || !hash(&after[0]) || !hash(last) {
        return Some(last_line);
    }
    let first_line = file.line_of(after[0].start);
    let anchored = if first_line == code_line {
        // An inline comment followed by more comments.
        after.len() > 1
    } else {
        // Own-line comments starting on the line after the last code line
        // (past its heredoc bodies), which ends in a newline token.
        let mut next = code_line + 1;
        while let Some(heredoc) = heredocs
            .iter()
            .find(|h| h.body.start == file.line_start(next) && h.opening.start < code + 1)
        {
            next = file.line_of(heredoc.closing.end.saturating_sub(1)) + 1;
        }
        first_line == next && file.line_text(code_line).last() != Some(&b'\\')
    };
    Some(if anchored { code_line } else { last_line })
}

#[cfg(test)]
mod tests;
