//! `Layout/LineLength` — flags lines longer than the limit: 120 columns,
//! or `max` in `rustocop.toml`, the one numeric setting
//! rustocop has.
//!
//! Follows RuboCop's cop at its default settings, which are fixed:
//!
//! - `AllowHeredoc: true`: lines in a heredoc's body are never too long.
//! - `AllowURI: true` (schemes `http` and `https`) and
//!   `AllowQualifiedName: true`: a line is accepted when the last URI (or
//!   qualified constant name, `Foo::Bar::baz`) on it starts before the limit
//!   and runs to the end of the line, together with the text stuck to it
//!   (closing quotes, brackets, or the rest of a `{…}` YARD link ending the
//!   line). When such a URI or name starts before the limit but something
//!   follows it, the offense starts after it.
//! - `AllowCopDirectives: true`: a directive comment (`# rubocop:…` or
//!   `# rustocop:…`) does not count toward the length.
//! - `AllowRBSInlineAnnotation: false` and no `AllowedPatterns`.
//! - A shebang on the first line, and the data after `__END__`, are not
//!   checked.
//!
//! Length is display width ([`unicode_width`]): wide characters (CJK, most
//! emoji) take two columns and combining marks none, where RuboCop counts
//! characters. Leading tabs take two columns each, as in RuboCop with its
//! default indentation width; other tabs one. The `\r` of a CRLF line
//! ending is not part of the line. The offense is reported at the first
//! character that runs past the limit.

use std::ops::Range;

use ruby_prism::Node;
use unicode_width::UnicodeWidthStr;

use crate::{
    cops::{
        Cop, NodeContext,
        walk::{NodeVisitor, walk},
    },
    directives,
    offense::Offense,
    source::SourceFile,
};

/// The `Layout/LineLength` cop.
pub struct LineLength {
    /// The longest line accepted, in columns.
    max: usize,
    /// A longer line is accepted when one of these matches it.
    allowed: Vec<regex::bytes::Regex>,
}

impl LineLength {
    /// The default limit.
    pub const DEFAULT_MAX: usize = 120;

    /// The cop with a limit of `max` columns.
    pub fn new(max: usize) -> Self {
        LineLength {
            max,
            allowed: Vec::new(),
        }
    }
}

impl Default for LineLength {
    fn default() -> Self {
        LineLength::new(LineLength::DEFAULT_MAX)
    }
}

/// The columns a leading tab takes: RuboCop's tab width when
/// `Layout/IndentationStyle` sets none, the default indentation width.
const TAB_WIDTH: usize = 2;

impl Cop for LineLength {
    fn name(&self) -> &'static str {
        "Layout/LineLength"
    }

    fn max_setting(&self) -> Option<&'static str> {
        Some("max")
    }

    fn with_max(&self, max: usize) -> Option<Box<dyn Cop>> {
        let allowed = self.allowed.clone();
        Some(Box::new(LineLength { max, allowed }))
    }

    fn with_allowed_patterns(&self, patterns: &[regex::bytes::Regex]) -> Option<Box<dyn Cop>> {
        Some(Box::new(LineLength {
            max: self.max,
            allowed: patterns.to_vec(),
        }))
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    /// A line-oriented check that runs once, at the root, rather than in
    /// `check_source`: the tree is at hand there, so heredoc bodies are only
    /// collected for files that have a line too long.
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_none() {
            return;
        }
        let mut heredoc_bodies = None;
        let last_line = u32::try_from(file.code_line_count()).unwrap_or(u32::MAX);
        for line in 1..=last_line {
            let text = file.line_text(line);
            if !may_exceed(text, self.max) {
                continue;
            }
            let measured = Measured::new(text);
            let length = measured.width();
            if length <= self.max || (line == 1 && text.starts_with(b"#!")) {
                continue;
            }
            if self.allowed.iter().any(|pattern| pattern.is_match(text)) {
                continue;
            }
            let bodies = heredoc_bodies.get_or_insert_with(|| heredoc_body_lines(node, file));
            if bodies.iter().any(|body| body.contains(&line)) {
                continue;
            }
            let Some((length, excess_start)) = self.excess(file, line, &measured, length) else {
                continue;
            };
            let (line, col) = file.line_col(file.line_start(line) + excess_start);
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: format!("Line is too long. [{length}/{}]", self.max),
                line,
                end_line: line,
                col,
            });
        }
    }
}

impl LineLength {
    /// For a line wider than the limit, the length to report and the byte
    /// offset where the excess starts; `None` when an exemption applies.
    /// RuboCop's `check_directive_line` and `check_line_for_exemptions`.
    fn excess(
        &self,
        file: &SourceFile,
        line: u32,
        measured: &Measured<'_>,
        length: usize,
    ) -> Option<(usize, usize)> {
        let text = measured.text;
        if has_directive_comment(file, line, text.len()) {
            let before = directives::marker_start(text).unwrap_or(text.len());
            let length = measured.width_to(rstrip(&text[..before]).len());
            return (length > self.max).then(|| (length, measured.crossing(self.max)));
        }
        let uri = self.excessive_range(measured, last_uri(text));
        let name = self.excessive_range(measured, last_qualified_name(text));
        let allowed = |range: &Excessive| range.begin < self.max && range.end == text.len();
        let accepted = match (&uri, &name) {
            (Some(uri), Some(name)) => allowed(uri) && allowed(name),
            (Some(range), None) | (None, Some(range)) => allowed(range),
            (None, None) => false,
        };
        if accepted {
            return None;
        }
        let excess_start = match uri.or(name) {
            Some(range) if range.begin < self.max => range.end,
            _ => measured.crossing(self.max),
        };
        Some((length, excess_start))
    }

    /// A URI or qualified name found at `found`, extended over the text
    /// stuck to it, unless it ends before the limit (RuboCop's
    /// `find_excessive_range`).
    fn excessive_range(
        &self,
        measured: &Measured<'_>,
        found: Option<Range<usize>>,
    ) -> Option<Excessive> {
        let found = found?;
        let end = extend_end(measured.text, found.end);
        let begin = measured.width_to(found.start);
        if begin < self.max && measured.width_to(end) < self.max {
            return None;
        }
        Some(Excessive { begin, end })
    }
}

/// Width shared with prospective modifier-condition rendering.
pub(crate) fn display_width(text: &[u8]) -> usize {
    Measured::new(text).width()
}

/// Existing modifier lines use StatementModifier's URI/directive exemptions,
/// not LineLength's additional qualified-name exemption or heredoc-body rules.
pub(crate) fn modifier_line_acceptable(file: &SourceFile, line: u32, max: usize) -> bool {
    let text = file.line_text(line);
    let measured = Measured::new(text);
    if measured.width() <= max || !file.line_length_enabled(line) {
        return true;
    }
    if has_directive_comment(file, line, text.len()) {
        let before = directives::marker_start(text).unwrap_or(text.len());
        return measured.width_to(rstrip(&text[..before]).len()) <= max;
    }
    LineLength::new(max)
        .excessive_range(&measured, last_uri(text))
        .is_some_and(|range| range.begin < max && range.end == text.len())
}

/// A URI or qualified name that reaches past the limit.
struct Excessive {
    /// The column it starts at (0-based).
    begin: usize,
    /// The byte offset it ends at, the text stuck to it included.
    end: usize,
}

/// Whether a line may be wider than `max` columns: no character takes
/// more columns than it has UTF-8 bytes, except a leading tab.
fn may_exceed(text: &[u8], max: usize) -> bool {
    text.len() > max
        || (text.first() == Some(&b'\t') && text.len() + leading_tabs(text) * (TAB_WIDTH - 1) > max)
}

fn leading_tabs(text: &[u8]) -> usize {
    text.iter().take_while(|&&byte| byte == b'\t').count()
}

/// A line, measured in display columns.
struct Measured<'a> {
    text: &'a [u8],
    /// The line, if it is UTF-8 and not all ASCII. ASCII lines, and lines
    /// that are not UTF-8, take one column per byte.
    wide: Option<&'a str>,
    leading_tabs: usize,
}

impl<'a> Measured<'a> {
    fn new(text: &'a [u8]) -> Self {
        let wide = if text.is_ascii() {
            None
        } else {
            std::str::from_utf8(text).ok()
        };
        Measured {
            text,
            wide,
            leading_tabs: leading_tabs(text),
        }
    }

    /// The columns the line takes.
    fn width(&self) -> usize {
        self.width_to(self.text.len())
    }

    /// The columns `text[..end]` takes; `end` is on a character boundary.
    fn width_to(&self, end: usize) -> usize {
        let base = match self.wide {
            Some(text) => text[..end].width(),
            None => end,
        };
        base + self.leading_tabs.min(end) * (TAB_WIDTH - 1)
    }

    /// The byte offset of the first character that runs past column `max`,
    /// on a line wider than that.
    fn crossing(&self, max: usize) -> usize {
        match self.wide {
            None => (0..self.text.len())
                .find(|&at| self.width_to(at + 1) > max)
                .unwrap_or(0),
            Some(text) => {
                // Bisect over character ends: widths of longer prefixes
                // are never smaller, ligatures aside.
                let ends: Vec<usize> = text
                    .char_indices()
                    .map(|(at, c)| at + c.len_utf8())
                    .collect();
                let first_past = ends.partition_point(|&end| self.width_to(end) <= max);
                first_past
                    .checked_sub(1)
                    .and_then(|previous| ends.get(previous))
                    .copied()
                    .unwrap_or(0)
            }
        }
    }
}

/// Whether a directive comment starts on `line` (RuboCop's
/// `directive_on_source_line?`).
fn has_directive_comment(file: &SourceFile, line: u32, line_len: usize) -> bool {
    let start = file.line_start(line);
    let comments = file.comments();
    let first = comments.partition_point(|comment| comment.start < start);
    comments
        .get(first)
        .filter(|comment| comment.start < start + line_len)
        .is_some_and(|comment| directives::is_directive_comment(&file.bytes[comment.clone()]))
}

/// `text` without trailing whitespace or NULs, like Ruby's `rstrip`.
fn rstrip(text: &[u8]) -> &[u8] {
    let end = text
        .iter()
        .rposition(|&byte| !is_ruby_space(byte) && byte != 0)
        .map_or(0, |at| at + 1);
    &text[..end]
}

/// Ruby's `\s`.
fn is_ruby_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\x0B' | b'\x0C' | b'\r')
}

/// Where the end of a URI or qualified name found ending at `end` is moved
/// to (RuboCop's `extend_end_position`): to the end of a `{…}` YARD link
/// ending the line, then on to the next whitespace.
fn extend_end(text: &[u8], mut end: usize) -> usize {
    if let Some((&b'}', before)) = text.split_last()
        && memchr::memchr(b'{', before).is_some()
        && let Some(close) = memchr::memrchr(b'}', &text[end..])
    {
        end += close + 1;
    }
    end + text[end..]
        .iter()
        .take_while(|&&byte| !is_ruby_space(byte))
        .count()
}

/// The last `http:` or `https:` URI on the line, as RuboCop's `match_uris`
/// finds them: scanning left to right, each match resuming after the
/// previous one, and keeping the matches Ruby's `URI.parse` accepts.
fn last_uri(text: &[u8]) -> Option<Range<usize>> {
    let mut resume = 0;
    let mut last = None;
    for start in memchr::memmem::find_iter(text, b"http") {
        if start < resume {
            continue;
        }
        let scheme_end = match &text[start + 4..] {
            [b':', ..] => start + 5,
            [b's', b':', ..] => start + 6,
            _ => continue,
        };
        let (end, valid) = uri_after_scheme(text, scheme_end);
        if valid {
            last = Some(start..end);
        }
        resume = end;
    }
    last
}

/// Where a URI whose `scheme:` ends at `at` ends, and whether Ruby's
/// `URI.parse` (RFC 3986) accepts it.
///
/// The extent follows the regexp RuboCop builds with
/// `URI::RFC2396_PARSER.make_regexp`: an opaque part, or an optional
/// `//authority`, absolute path and `?query`; then an optional `#fragment`.
/// Of what that matches, RFC 3986 rejects brackets in the fragment, in an
/// opaque part before its query, and around a host that is no IPv6
/// address.
fn uri_after_scheme(text: &[u8], mut at: usize) -> (usize, bool) {
    let mut valid = true;
    if text
        .get(at)
        .is_some_and(|&byte| is_uri_unreserved(byte) || b";?:@&=+$,".contains(&byte))
        || is_escape(text, at)
    {
        let opaque = at;
        at = uri_run(text, at, is_uric);
        let path = &text[opaque..at];
        let path = &path[..memchr::memchr(b'?', path).unwrap_or(path.len())];
        valid = !has_bracket(path);
    } else {
        if text[at..].starts_with(b"//") {
            let authority;
            (authority, valid) = uri_authority(text, at + 2);
            at = authority;
        }
        if text.get(at) == Some(&b'/') {
            at = uri_run(text, at, |byte| {
                is_uri_unreserved(byte) || b":@&=+$,;/".contains(&byte)
            });
        }
        if text.get(at) == Some(&b'?') {
            at = uri_run(text, at + 1, is_uric);
        }
    }
    if text.get(at) == Some(&b'#') {
        let fragment = at + 1;
        at = uri_run(text, fragment, is_uric);
        valid &= !has_bracket(&text[fragment..at]);
    }
    (at, valid)
}

/// The end of an RFC 2396 authority starting at `at` (`[userinfo@]host[:port]`,
/// possibly empty), and whether RFC 3986 accepts its host.
fn uri_authority(text: &[u8], at: usize) -> (usize, bool) {
    let userinfo_end = uri_run(text, at, |byte| {
        is_uri_unreserved(byte) || b";:&=+$,".contains(&byte)
    });
    let host = if text.get(userinfo_end) == Some(&b'@') {
        uri_host(text, userinfo_end + 1).or_else(|| uri_host(text, at))
    } else {
        uri_host(text, at)
    };
    let Some((mut end, valid)) = host else {
        return (at, true);
    };
    if text.get(end) == Some(&b':') {
        end += 1 + text[end + 1..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
    }
    (end, valid)
}

/// The end of an RFC 2396 host starting at `at` (a host name, or an IPv6
/// address in brackets), and whether RFC 3986 accepts it.
fn uri_host(text: &[u8], at: usize) -> Option<(usize, bool)> {
    let name_end = uri_run(text, at, |byte| {
        byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.'
    });
    if name_end > at {
        return Some((name_end, true));
    }
    if text.get(at) != Some(&b'[') {
        return None;
    }
    let address = &text[at + 1..];
    let len = address
        .iter()
        .take_while(|&&byte| byte.is_ascii_hexdigit() || byte == b':' || byte == b'.')
        .count();
    if len == 0 || address.get(len) != Some(&b']') {
        return None;
    }
    let valid = std::str::from_utf8(&address[..len])
        .is_ok_and(|address| address.parse::<std::net::Ipv6Addr>().is_ok());
    Some((at + len + 2, valid))
}

/// The end of a run of bytes matching `accepts`, or `%XX` escapes, from
/// `at`.
fn uri_run(text: &[u8], mut at: usize, accepts: impl Fn(u8) -> bool) -> usize {
    while let Some(&byte) = text.get(at) {
        if is_escape(text, at) {
            at += 3;
        } else if accepts(byte) {
            at += 1;
        } else {
            break;
        }
    }
    at
}

fn is_escape(text: &[u8], at: usize) -> bool {
    text.get(at) == Some(&b'%')
        && text
            .get(at + 1..at + 3)
            .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
}

/// RFC 2396 `unreserved`.
fn is_uri_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte)
}

/// RFC 2396 `uric` (Ruby's, with RFC 2732's brackets), escapes aside.
fn is_uric(byte: u8) -> bool {
    is_uri_unreserved(byte) || b";/?:@&=+$,[]".contains(&byte)
}

fn has_bracket(text: &[u8]) -> bool {
    text.iter().any(|&byte| byte == b'[' || byte == b']')
}

/// The last qualified name on the line, as RuboCop's
/// `match_qualified_names` finds them with
/// `/\b(?:[A-Z][A-Za-z0-9_]*::)+[A-Za-z_][A-Za-z0-9_]*\b/`.
fn last_qualified_name(text: &[u8]) -> Option<Range<usize>> {
    memchr::memmem::find(text, b"::")?;
    let mut at = 0;
    let mut last = None;
    while at < text.len() {
        match qualified_name_at(text, at) {
            Some(end) => {
                last = Some(at..end);
                at = end;
            }
            None => at += 1,
        }
    }
    last
}

/// The end of the qualified name starting at `start`, if one does.
fn qualified_name_at(text: &[u8], start: usize) -> Option<usize> {
    if !text[start].is_ascii_uppercase() || is_word_before(text, start) {
        return None;
    }
    // Take segments greedily: a segment is followed by another while it is
    // capitalized and a `::` and a letter or `_` come next.
    let segment_end = |segment: usize| {
        segment
            + 1
            + text[segment + 1..]
                .iter()
                .take_while(|&&byte| is_ascii_word(byte))
                .count()
    };
    let (mut segment, mut segments) = (start, 1);
    let (mut previous_end, mut end) = (start, segment_end(start));
    while text[segment].is_ascii_uppercase()
        && text[end..].starts_with(b"::")
        && text
            .get(end + 2)
            .is_some_and(|&byte| byte.is_ascii_alphabetic() || byte == b'_')
    {
        segment = end + 2;
        segments += 1;
        previous_end = end;
        end = segment_end(segment);
    }
    // The closing `\b` fails before a non-ASCII word character; the regexp
    // then backtracks to end at the previous segment.
    if is_word_at(text, end) {
        segments -= 1;
        end = previous_end;
    }
    (segments >= 2).then_some(end)
}

fn is_ascii_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Whether the character ending at `at` is a word character for a Ruby
/// regexp's `\b` (which, unlike `\w`, knows non-ASCII letters).
fn is_word_before(text: &[u8], at: usize) -> bool {
    let start = (at.saturating_sub(4)..at)
        .find(|&start| std::str::from_utf8(&text[start..at]).is_ok_and(|s| s.chars().count() == 1))
        .unwrap_or(at);
    is_word_char(&text[start..at])
}

/// Whether the character starting at `at` is a word character for `\b`.
fn is_word_at(text: &[u8], at: usize) -> bool {
    let end = (at + 1..=(at + 4).min(text.len()))
        .find(|&end| std::str::from_utf8(&text[at..end]).is_ok())
        .unwrap_or(at);
    is_word_char(&text[at..end])
}

fn is_word_char(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.chars().next())
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// The lines of every heredoc's body, as ranges of 1-based line numbers.
fn heredoc_body_lines(root: &Node<'_>, file: &SourceFile) -> Vec<Range<u32>> {
    if !file.contains(b"<<") {
        return Vec::new();
    }
    let mut collector = HeredocBodies {
        file,
        lines: Vec::new(),
    };
    walk(&mut collector, root);
    collector.lines
}

/// Collects heredoc bodies.
struct HeredocBodies<'a> {
    file: &'a SourceFile,
    lines: Vec<Range<u32>>,
}

impl<'pr> NodeVisitor<'pr> for HeredocBodies<'_> {
    fn enter(&mut self, node: Node<'pr>) {
        if let Some(lines) = heredoc_body(&node, self.file) {
            self.lines.push(lines);
        }
    }
}

/// The lines of a heredoc's body: from its first line up to, not
/// including, the terminator's.
fn heredoc_body(node: &Node<'_>, file: &SourceFile) -> Option<Range<u32>> {
    let first_part = |parts: ruby_prism::NodeList<'_>| {
        parts
            .iter()
            .next()
            .map(|part| part.location().start_offset())
    };
    let (opening, body, closing) = match node {
        Node::StringNode { .. } => {
            let string = node.as_string_node()?;
            (
                string.opening_loc()?,
                Some(string.content_loc().start_offset()),
                string.closing_loc()?,
            )
        }
        Node::XStringNode { .. } => {
            let string = node.as_x_string_node()?;
            (
                string.opening_loc(),
                Some(string.content_loc().start_offset()),
                string.closing_loc(),
            )
        }
        Node::InterpolatedStringNode { .. } => {
            let string = node.as_interpolated_string_node()?;
            (
                string.opening_loc()?,
                first_part(string.parts()),
                string.closing_loc()?,
            )
        }
        Node::InterpolatedXStringNode { .. } => {
            let string = node.as_interpolated_x_string_node()?;
            (
                string.opening_loc(),
                first_part(string.parts()),
                string.closing_loc(),
            )
        }
        _ => return None,
    };
    if !opening.as_slice().starts_with(b"<<") {
        return None;
    }
    Some(file.line_of(body?)..file.line_of(closing.start_offset()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{lint_str, lint_str_with};

    const NAME: &str = "Layout/LineLength";

    /// (line, col, message) of each offense with a limit of `max`.
    fn found_with(max: usize, source: &str) -> Vec<(u32, u32, String)> {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert!(
            parsed.errors().next().is_none(),
            "fixture must parse: {source:?}"
        );
        lint_str_with(&[&LineLength::new(max)], source)
            .into_iter()
            .map(|o| {
                assert_eq!(o.cop, NAME);
                (o.line, o.col, o.message)
            })
            .collect()
    }

    /// (line, col, message) of each offense with the default config.
    fn found(source: &str) -> Vec<(u32, u32, String)> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == NAME)
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    /// An expected offense.
    fn at(line: u32, col: u32, length: usize, max: usize) -> (u32, u32, String) {
        (line, col, format!("Line is too long. [{length}/{max}]"))
    }

    #[test]
    fn flags_a_line_over_120_columns_at_the_first_column_past_it() {
        let source = format!("x = 1\n{}\n", "a".repeat(121));
        assert_eq!(found(&source), [at(2, 121, 121, 120)]);
        let source = format!("# {}\n", "a".repeat(140));
        assert_eq!(found(&source), [at(1, 121, 142, 120)]);
    }

    #[test]
    fn accepts_lines_up_to_120_columns() {
        assert!(found(&format!("{}\n", "a".repeat(120))).is_empty());
        assert!(found(&format!("x = 1 # {}\n", "a".repeat(112))).is_empty());
        assert!(found("").is_empty());
    }

    #[test]
    fn reports_as_a_convention() {
        let offenses = lint_str(&format!("{}\n", "a".repeat(121)));
        let offense = offenses.iter().find(|o| o.cop == NAME).unwrap();
        assert_eq!(offense.severity, crate::offense::Severity::Convention);
    }

    #[test]
    fn measures_wide_characters_by_display_width() {
        // 2 + 59 * 2 = 120 columns, 61 characters.
        assert!(found(&format!("# {}\n", "日".repeat(59))).is_empty());
        // 122 columns: the 61st wide character ends past 120.
        assert_eq!(
            found(&format!("# {}\n", "日".repeat(60))),
            [at(1, 62, 122, 120)]
        );
        assert_eq!(
            found(&format!("x = \"{}\"\n", "😀".repeat(60))),
            [at(1, 63, 126, 120)]
        );
        // A full-width space is wide too.
        assert_eq!(
            found(&format!("#{}\n", "\u{3000}".repeat(60))),
            [at(1, 61, 121, 120)]
        );
    }

    #[test]
    fn measures_combining_marks_as_zero_width() {
        // 140 characters, 70 columns.
        assert!(found(&format!("# {}\n", "e\u{301}".repeat(70))).is_empty());
        // Multibyte but narrow: one column each.
        assert!(found(&format!("# {}\n", "é".repeat(118))).is_empty());
        assert_eq!(
            found(&format!("# {}\n", "é".repeat(119))),
            [at(1, 121, 121, 120)]
        );
    }

    #[test]
    fn ignores_the_carriage_return_of_crlf_lines() {
        let source = format!("{}\r\nx = 1\r\n", "a".repeat(120));
        assert!(found(&source).is_empty());
        let source = format!("x = 1\r\n{}\r\n", "a".repeat(121));
        assert_eq!(found(&source), [at(2, 121, 121, 120)]);
    }

    #[test]
    fn counts_leading_tabs_as_two_columns() {
        let indented =
            |rest: usize| format!("if x\n{}{}\nend\n", "\t".repeat(10), "a".repeat(rest));
        assert!(found(&indented(100)).is_empty());
        assert_eq!(found(&indented(101)), [at(2, 111, 121, 120)]);
        // A tab after the indentation takes one column.
        assert!(found(&format!("x = 1 #\t{}\n", "a".repeat(112))).is_empty());
    }

    #[test]
    fn honors_the_configured_max() {
        assert_eq!(
            found_with(80, &format!("{}\n", "a".repeat(81))),
            [at(1, 81, 81, 80)]
        );
        assert!(found_with(130, &format!("{}\n", "a".repeat(130))).is_empty());
        assert_eq!(
            found_with(1, "ab\n# \u{301}\n"),
            [at(1, 2, 2, 1), at(2, 2, 2, 1)]
        );
    }

    #[test]
    fn skips_a_shebang_on_the_first_line_only() {
        let long = format!("#!/usr/bin/env ruby {}", "a".repeat(120));
        assert!(found(&format!("{long}\nx = 1\n")).is_empty());
        assert_eq!(found(&format!("x = 1\n{long}\n")), [at(2, 121, 140, 120)]);
    }

    #[test]
    fn skips_the_data_after_end() {
        let long = "a".repeat(121);
        assert_eq!(
            found(&format!("{long}\n__END__\n{long}\n")),
            [at(1, 121, 121, 120)]
        );
        // `__END__` in a heredoc is text.
        assert_eq!(
            found(&format!("x = <<~A\n__END__\nA\n{long}\n")),
            [at(4, 121, 121, 120)]
        );
    }

    #[test]
    fn skips_heredoc_bodies() {
        let long = "a".repeat(130);
        for source in [
            format!("x = <<~SQL\n  {long}\nSQL\n"),
            format!("x = <<-SQL\n  {long}\n  SQL\n"),
            format!("x = <<SQL\n{long}\nSQL\n"),
            format!("x = <<~'SQL'\n  {long}\nSQL\n"),
            format!("x = <<~SQL\n  #{{y}} {long}\nSQL\n"),
            format!("x = <<~CMD.strip\n  #{{y}}\n  {long}\nCMD\n"),
            format!("x = <<~`CMD`\n  {long}\nCMD\n"),
            format!("x = <<~`CMD`\n  #{{y}} {long}\nCMD\n"),
            format!("foo(<<~A, <<~B)\n  {long}\nA\n  {long}\nB\n"),
            format!("x = <<~A\n  #{{<<~B}}\n    {long}\n  B\n  {long}\nA\n"),
        ] {
            assert!(found(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn checks_the_lines_around_heredoc_bodies() {
        let long = "a".repeat(121);
        assert_eq!(
            found(&format!("foo(<<~A, {long})\n  body\nA\n")),
            [at(1, 121, 132, 120)]
        );
        // A plain multi-line string is not a heredoc.
        assert_eq!(
            found(&format!("x = \"\n{long}\n\"\n")),
            [at(2, 121, 121, 120)]
        );
        assert_eq!(
            found(&format!("=begin\n{long}\n=end\n")),
            [at(2, 121, 121, 120)]
        );
        // An empty heredoc covers no line.
        assert_eq!(
            found(&format!("x = <<~A\nA\n{long}\n")),
            [at(3, 121, 121, 120)]
        );
    }

    #[test]
    fn does_not_count_directive_comments() {
        let code = format!("x = {}", "a".repeat(100));
        for directive in [
            "# rubocop:disable Style/Semicolon",
            "# rustocop:disable Style/Semicolon",
            "# rubocop:todo Style/Semicolon -- a reason that is long enough",
        ] {
            assert!(
                found(&format!("{code} {directive}\n")).is_empty(),
                "{directive}"
            );
        }
        // The code itself still counts; the offense ignores the comment.
        let code = format!("x = {}", "a".repeat(120));
        assert_eq!(
            found(&format!("{code} # rubocop:disable Style/Semicolon\n")),
            [at(1, 121, 124, 120)]
        );
        // A commented-out directive is an ordinary comment.
        let code = format!("x = {}", "a".repeat(100));
        assert_eq!(
            found(&format!("{code} # # rubocop:disable Style/Semicolon\n")),
            [at(1, 121, 140, 120)]
        );
    }

    #[test]
    fn a_line_length_directive_suppresses_the_offense() {
        let long = "a".repeat(121);
        assert!(found(&format!("# rubocop:disable Layout/LineLength\n{long}\n")).is_empty());
        assert!(found(&format!("# rustocop:disable Layout\n{long}\n")).is_empty());
    }

    #[test]
    fn accepts_a_long_uri_or_qualified_name_ending_the_line() {
        let pad = "x".repeat(60);
        for source in [
            format!("# {pad} https://example.com/{pad}\n"),
            format!("url = \"https://example.com/{pad}{pad}\"\n"),
            format!("# See {{https://example.com/{pad}{pad} Title}}\n"),
            format!("# {pad} Foo::Bar::{pad}\n"),
            format!("x = Foo::Bar::{pad}{pad}.new\n"),
        ] {
            assert!(found(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn reports_the_text_after_a_long_uri() {
        let pad = "x".repeat(60);
        let source = format!("# {pad} https://example.com/{pad} and more\n");
        // The URI ends at byte 143; ` and more` is reported.
        assert_eq!(found(&source), [at(1, 144, 152, 120)]);
        // A URI starting past the limit does not help.
        let source = format!("# {pad}{pad} https://example.com/\n");
        assert_eq!(found(&source), [at(1, 121, 143, 120)]);
        // Nor does a URI with another scheme, or upper case.
        let source = format!("# {pad} ftp://example.com/{pad}\n");
        assert_eq!(found(&source), [at(1, 121, 141, 120)]);
        let source = format!("# {pad} HTTPS://EXAMPLE.COM/{pad}\n");
        assert_eq!(found(&source), [at(1, 121, 143, 120)]);
    }

    #[test]
    fn counts_bytes_on_lines_that_are_not_utf8() {
        // Prism accepts invalid UTF-8 in a comment; each byte takes a column,
        // as `SourceFile::line_col` counts them.
        let mut source = b"# encoding: binary\n# ".to_vec();
        source.extend([0xff; 125]);
        source.extend(b"\nx = \"\xfe\"\n");
        let config = crate::config::ResolvedConfig::default();
        let report = crate::pipeline::lint_one(
            std::path::Path::new("x.rb"),
            std::path::Path::new("x.rb"),
            Some(&source),
            &[&LineLength::default()],
            &config,
        );
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.internal_errors.is_empty());
        let found: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| (o.line, o.col, o.message.clone()))
            .collect();
        assert_eq!(found, [at(2, 121, 127, 120)]);
    }

    #[test]
    fn finds_qualified_names_like_rubocops_regexp() {
        assert_eq!(last_qualified_name(b"Foo::Bar"), Some(0..8));
        assert_eq!(last_qualified_name(b"x Foo::Bar::baz y"), Some(2..15));
        assert_eq!(last_qualified_name(b"Foo::bar::Baz"), Some(0..8));
        assert_eq!(last_qualified_name(b"Foo::Bar::1"), Some(0..8));
        assert_eq!(last_qualified_name(b"A::B C::D"), Some(5..9));
        assert_eq!(last_qualified_name(b"xFoo::Bar::Baz"), Some(6..14));
        assert_eq!(last_qualified_name(b"Foo:::Bar"), None);
        assert_eq!(last_qualified_name(b"foo::Bar"), None);
        assert_eq!(last_qualified_name(b"Foo"), None);
        // `\b` knows non-ASCII letters.
        assert_eq!(last_qualified_name("éFoo::Bar".as_bytes()), None);
        assert_eq!(last_qualified_name("Foo::Baré".as_bytes()), None);
        assert_eq!(last_qualified_name("Foo::Bar::Bazé".as_bytes()), Some(0..8));
        assert_eq!(last_qualified_name("Foo::Baré::Baz".as_bytes()), None);
        assert_eq!(last_qualified_name("日Foo::Bar".as_bytes()), None);
        assert_eq!(last_qualified_name(b"_Foo::Bar"), None);
        assert_eq!(last_qualified_name(b"1Foo::Bar"), None);
        assert_eq!(last_qualified_name(b"::Foo::Bar"), Some(2..10));
        assert_eq!(last_qualified_name(b"Foo::_bar"), Some(0..9));
        assert_eq!(last_qualified_name(b"A::B::C::d::E"), Some(0..10));
        // A byte that is not UTF-8 is no word character.
        assert_eq!(last_qualified_name(b"\xffFoo::Bar\xff"), Some(1..9));
    }

    #[test]
    fn finds_uris_like_rubocops_regexp() {
        assert_eq!(last_uri(b"see http://a.com/x y"), Some(4..18));
        assert_eq!(last_uri(b"'https://a.com/x'"), Some(1..17));
        assert_eq!(last_uri(b"\"https://a.com/x\""), Some(1..16));
        assert_eq!(last_uri(b"http://a.com https://b.com"), Some(13..26));
        // A URI swallows the one in its query.
        assert_eq!(last_uri(b"http://a.com/?u=http://b.com"), Some(0..28));
        assert_eq!(last_uri(b"http://a.com/#x#y"), Some(0..15));
        assert_eq!(last_uri(b"http://a.com/%zz"), Some(0..13));
        assert_eq!(last_uri(b"httpx://a.com"), None);
        assert_eq!(last_uri(b"xhttp://a.com"), Some(1..13));
        // The path stops at a bracket; the query takes it.
        assert_eq!(last_uri(b"http://a.com/x[1]"), Some(0..14));
        assert_eq!(last_uri(b"http://a.com/x?y[1]=2"), Some(0..21));
        assert_eq!(last_uri(b"http://u:p@a.com:80/x"), Some(0..21));
        assert_eq!(last_uri(b"http://[::1]:80/x"), Some(0..17));
        assert_eq!(last_uri(b"http://a_b.com/x"), Some(0..8));
        assert_eq!(last_uri(b"http://user@/x"), Some(0..11));
        assert_eq!(last_uri(b"http://[1:2]/x"), None);
        assert_eq!(last_uri(b"http::foo"), Some(0..9));
        assert_eq!(last_uri(b"see http: there"), Some(4..9));
        assert_eq!(last_uri(b"http:foo?a[1]"), Some(0..13));
    }

    #[test]
    fn skips_uris_that_ruby_cannot_parse() {
        // RuboCop keeps the matches `URI.parse` (RFC 3986) accepts: no
        // brackets in a fragment, or in an opaque URI before its query.
        assert_eq!(last_uri(b"http://a.com/#x]:"), None);
        assert_eq!(last_uri(b"http:foo[1]"), None);
        // The scan still resumes after the rejected match.
        assert_eq!(last_uri(b"http://a.com/#x]http://b.com"), None);
        assert_eq!(last_uri(b"http://a.com/#x] http://b.com"), Some(17..29));
        let pad = "x".repeat(60);
        let source = format!("# see {{Header}}[https://example.com/{pad}#{pad}]:\n");
        assert_eq!(found(&source), [at(1, 121, 158, 120)]);
    }

    mod rubocop_spec {
        //! Examples ported from RuboCop's
        //! `spec/rubocop/cop/layout/line_length_spec.rb`
        //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov): those whose
        //! settings match rustocop's fixed ones, with the example's `Max`.
        //! The spec turns every `Allow*` option off unless a context turns
        //! it on; examples are kept where that makes no difference.

        use super::*;

        #[test]
        fn basics() {
            let max = "#".repeat(80);
            assert_eq!(found_with(80, &format!("{max}#\n")), [at(1, 81, 81, 80)]);
            assert_eq!(found_with(80, &format!("{max}abc\n")), [at(1, 81, 83, 80)]);
            assert!(found_with(80, &max).is_empty());
            assert!(
                found_with(
                    80,
                    "#!/System/Library/Frameworks/Ruby.framework/Versions/2.3/usr/bin/ruby --disable-gems\n\ndo_something\n"
                )
                .is_empty()
            );
            let source = format!("{}\n__END__\n{}\n", "#".repeat(150), "#".repeat(200));
            assert_eq!(found_with(80, &source), [at(1, 81, 150, 80)]);
        }

        #[test]
        fn tab_indentation() {
            assert!(found_with(10, "\t\t\t123").is_empty());
            // RuboCop reports column 1, a stopgap (per its source) for lines
            // whose tabs alone run past the limit; rustocop reports the tab
            // that does.
            let source = format!("{}1\n", "\t".repeat(12));
            assert_eq!(found_with(10, &source), [at(1, 6, 25, 10)]);
        }

        const COMMIT_URL: &str =
            "https://github.com/rubocop/rubocop/commit/3b48d8bdf5b1c2e05e35061837309890f04ab08c";

        #[test]
        fn allow_uri() {
            let pad = " ".repeat(10);
            let source = format!(
                "{pad}# Some documentation comment...\n\
                 {pad}# See: https://github.com/rubocop/rubocop and then words that are not part of a URL\n"
            );
            assert_eq!(found_with(80, &source), [at(2, 81, 93, 80)]);
            for source in [
                format!("{pad}# Some documentation comment...\n{pad}# See: {COMMIT_URL}\n"),
                format!("{pad}  # See: '{COMMIT_URL}'\n"),
                format!("{pad}  # See: \"{COMMIT_URL}\"\n"),
                format!("{pad}  # See: {{{COMMIT_URL}}}\n"),
                format!("{pad}  # See: {{{COMMIT_URL} Optional Title}}\n"),
            ] {
                assert!(found_with(80, &source).is_empty(), "{source}");
            }
            let source = format!(
                "{pad}# See: http://google.com/, http://gmail.com/, https://maps.google.com/, http://plus.google.com/\n"
            );
            assert_eq!(found_with(80, &source), [at(1, 81, 105, 80)]);
            let source = format!("{pad}# See: {COMMIT_URL} and\n{pad}#   http://google.com/\n");
            assert_eq!(found_with(80, &source), [at(1, 100, 103, 80)]);
            let source =
                format!("{pad}# See: \"{COMMIT_URL}\" and\n{pad}#   \"http://google.com/\"\n");
            assert_eq!(found_with(80, &source), [at(1, 102, 105, 80)]);
            let source = format!("{pad}# See: {{{COMMIT_URL}}} and\n{pad}#   http://google.com/\n");
            assert_eq!(found_with(80, &source), [at(1, 102, 105, 80)]);
            let source = format!("{pad}# See: {COMMIT_URL} \n{pad}#   http://google.com/\n");
            assert_eq!(found_with(80, &source), [at(1, 100, 100, 80)]);
            let source = "\t\t\t\t# There is some content http://test.com\n";
            assert_eq!(found_with(30, source), [at(1, 27, 47, 30)]);
            let x = "x".repeat(40);
            let source = format!(
                "{x} = 'otherprotocol://a.very.long.line.which.violates.LineLength/sadf'\n"
            );
            assert_eq!(found_with(80, &source), [at(1, 81, 108, 80)]);
            let uri = "https://a.very.long.line.which.violates.LineLength/sadf";
            for source in [
                format!("{x} = '{uri}'\n{x} = \"{uri}\"\n"),
                format!("{x}(\"{uri}\")\n{x} \"{uri}\"\n{x}('{uri}')\n{x} '{uri}'\n"),
                format!("# See: {COMMIT_URL}\n"),
            ] {
                assert!(found_with(80, &source).is_empty(), "{source}");
            }
            // The scheme's case must match.
            let source = "# See: HTTPS://GITHUB.COM/RUBOCOP/RUBOCOP/COMMIT/3B48D8BDF5B1C2E05E35061837309890F04AB08C\n";
            assert_eq!(found_with(80, source), [at(1, 81, 89, 80)]);
        }

        const QUALIFIED: &str =
            "ActiveRecord::ConnectionAdapters::OracleEnhanced::SchemaStatements::create_table";

        #[test]
        fn allow_qualified_name() {
            let pad = " ".repeat(10);
            let source = format!(
                "{pad}# invoke the normal migration method\n\
                 {pad}# Should call ActiveRecord::Oracle::SchemaStatements::create_table in the end\n"
            );
            assert_eq!(found_with(80, &source), [at(2, 81, 87, 80)]);
            for source in [
                format!(
                    "{pad}# invoke the normal migration method\n{pad}# should end up calling {QUALIFIED}\n"
                ),
                format!("{pad}  # should end up calling '{QUALIFIED}'\n"),
                format!("{pad}  # should end up calling \"{QUALIFIED}\"\n"),
                format!("{pad}  # should end up calling {{{QUALIFIED}}}\n"),
            ] {
                assert!(found_with(80, &source).is_empty(), "{source}");
            }
            let source = format!(
                "{pad}# Invoke the normal migration method, in oracle envs should end up calling ActiveRecord::Oracle::create_table\n"
            );
            assert_eq!(found_with(80, &source), [at(1, 81, 119, 80)]);
            let source = format!(
                "{pad}# Refer ActiveRecord::Migrations, in oracle envs should end up calling ActiveRecord::Oracle::create_table\n"
            );
            assert_eq!(found_with(80, &source), [at(1, 81, 115, 80)]);
            let source =
                format!("{pad}# Should call {QUALIFIED} and\n{pad}#   ActiveRecord::Example\n");
            assert_eq!(found_with(80, &source), [at(1, 105, 108, 80)]);
            let source = format!(
                "{pad}# Should call \"{QUALIFIED}\" and\n{pad}#   \"ActiveRecord::Example\"\n"
            );
            assert_eq!(found_with(80, &source), [at(1, 107, 110, 80)]);
            let source = format!(
                "{pad}# Should call {{{QUALIFIED}}} and\n{pad}#   {{ActiveRecord::Example}}\n"
            );
            assert_eq!(found_with(80, &source), [at(1, 107, 110, 80)]);
            let source = format!("{pad}# Should call {QUALIFIED} \n{pad}#   http://google.com/\n");
            assert_eq!(found_with(80, &source), [at(1, 105, 105, 80)]);
            let x = "x".repeat(40);
            let name = "ActiveRecord::Oracle::SchemaStatements::create_table";
            let source = format!("{x}(\"{name}\")\n{x} \"{name}\"\n{x}('{name}')\n{x} '{name}'\n");
            assert!(found_with(80, &source).is_empty());
        }

        #[test]
        fn allow_heredoc() {
            for source in [
                "<<-SQL\n  SELECT posts.id, posts.title, users.name FROM posts LEFT JOIN users ON posts.user_id = users.id;\nSQL\n",
                "<<~MESSAGE\n  #{'hello' * 1} #{'world' * 2} #{'hello' * 1} #{'world' * 2} #{'hello' * 1} #{'world' * 2}\nMESSAGE\n",
                "# this results in AST being nil",
            ] {
                assert!(found_with(80, source).is_empty(), "{source}");
            }
        }

        #[test]
        fn rbs_inline_annotations_count() {
            assert_eq!(
                found_with(10, "#: () -> String\ndef hash\nend\n"),
                [at(1, 11, 15, 10)]
            );
            assert_eq!(
                found_with(10, "def each_address(&block) #: void\nend\n"),
                [at(1, 11, 32, 10)]
            );
        }

        #[test]
        fn allow_cop_directives() {
            for source in [
                "# rubocop:disable Metrics/SomeReallyLongMetricNameThatShouldBeMuchShorterAndNeedsANameChange\n",
                "def method_definition_that_is_just_under_the_line_length_limit(foo, bar) # rubocop:disable Metrics/AbcSize\n  # complex method\nend\n",
                "def method_definition_that_is_just_under_the_line_length_limit(foo) # rubocop:disable Metrics/AbcSize inherently complex!\n  # complex\nend\n",
            ] {
                assert!(found_with(80, source).is_empty(), "{source}");
            }
            let source = format!("{}bcd # rubocop:enable Style/ClassVars\n", "a".repeat(80));
            assert_eq!(found_with(80, &source), [at(1, 81, 83, 80)]);
            let source = format!(
                "{} # bbbbbbbbbbbbbb # rubocop:enable Style/ClassVars'\n",
                "a".repeat(70)
            );
            assert_eq!(found_with(80, &source), [at(1, 81, 87, 80)]);
            let source = format!(
                "{}{}\n",
                " ".repeat(12),
                r"LARGE_DATA_STRING_PATTERN = %r{\A([A-Za-z0-9+/#]*={0,2})#([A-Za-z0-9+/#]*={0,2})#([A-Za-z0-9+/#]*={0,2})\z} # rubocop:disable Style/ClassVars"
            );
            assert_eq!(found_with(80, &source), [at(1, 81, 119, 80)]);
        }

        #[test]
        fn short_and_large_sources() {
            assert!(
                found_with(80, "if __FILE__ == $PROGRAM_NAME\n  do_something\nend\n").is_empty()
            );
            let rows: Vec<String> = (0..10_000)
                .map(|n| format!("['{n:04X}', 0x{n:04X}],"))
                .collect();
            let source = format!("[\n  {}\n]\n", rows.join("\n  "));
            assert!(found_with(80, &source).is_empty());
        }

        #[test]
        fn tabs_with_indentation_width_two() {
            let source = format!("\t\t{}a\n", "#".repeat(28));
            assert_eq!(found_with(30, &source), [at(1, 29, 33, 30)]);
            for source in [
                format!("\t{}", "#".repeat(28)),
                "\t\t# https://github.com/rubocop/rubocop".to_string(),
                "\t\t# https://github.com/ruboco".to_string(),
                "\t\t# See https://github.com/rubocop/rubocop".to_string(),
                "\t\t# See 'https://github.com/rubocop/rubocop'".to_string(),
            ] {
                assert!(found_with(30, &source).is_empty(), "{source:?}");
            }
            let source = format!(
                "{}\texpect(some_exception_variable) {{|e| e.url.should == 'http://host/path'}}\n",
                " ".repeat(10)
            );
            assert_eq!(found_with(30, &source), [at(1, 31, 83, 30)]);
        }

        #[test]
        fn autocorrection_examples() {
            // Only the offenses: rustocop does not autocorrect.
            for (source, length) in [
                ("'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaabbbbb'\n".to_string(), 43),
                (
                    r#"x = "\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13""#.to_string() + "\n",
                    82,
                ),
                (format!("{}'aaaa'\n", " ".repeat(40)), 46),
                ("%{aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}\n".to_string(), 44),
                ("%q[aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa]\n".to_string(), 45),
                ("%W[aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa]\n".to_string(), 45),
                (
                    "'aaaaaaaaaaaaaaaaaaaaa' # this comment makes the line too long\n".to_string(),
                    62,
                ),
                (
                    "{ x: 'aaaa', y: 'bbbbbbbbbbbbbbbbbbbbbbbbbbb', z: 'cccccccccccccccccccccccccccccccccccccccccc' }\n".to_string(),
                    96,
                ),
            ] {
                assert_eq!(found_with(40, &source), [at(1, 41, length, 40)], "{source}");
            }
            let source = format!("foo\n{}'aaaa'\n", " ".repeat(37));
            assert_eq!(found_with(40, &source), [at(2, 41, 43, 40)]);
            let source = "foo(\n  'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbb'\n)\n";
            assert_eq!(found_with(40, source), [at(2, 41, 46, 40)]);
            // RuboCop counts characters: [44/40] at column 41.
            let source = "\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaあbbbbb\"\n";
            assert_eq!(found_with(40, source), [at(1, 40, 45, 40)]);
        }
    }
}
