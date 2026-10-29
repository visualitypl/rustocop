//! `Style/RedundantRegexpEscape` — redundant punctuation escapes in regexps.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
use std::ops::Range;

pub struct RedundantRegexpEscape;
impl Cop for RedundantRegexpEscape {
    fn name(&self) -> &'static str {
        "Style/RedundantRegexpEscape"
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
        let Some(regexp) = super::regexp_literal::Regexp::new(node) else {
            return;
        };
        let interpolated = matches!(
            node,
            Node::InterpolatedRegularExpressionNode { .. }
                | Node::InterpolatedMatchLastLineNode { .. }
        );
        let mut scanner = Scanner::new(file, regexp.closing.as_slice().contains(&b'x'));
        let open = *regexp.opening.as_slice().last().unwrap();
        let close = regexp.closing.as_slice()[0];
        for fragment in regexp.fragments {
            scanner.scan(fragment.start_offset()..fragment.end_offset(), open, close);
        }
        let unobserved = cx.corrections_requested()
            && !scanner.escapes.is_empty()
            && super::exact_regexp_match::spelling_value_is_unobserved(node, cx, file);
        for (at, safe) in scanner.escapes {
            let (line, col) = file.line_col(at);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "Redundant escape inside regexp literal".into(),
                line,
                col,
                end_line: line,
                correction: if unobserved && !interpolated && safe {
                    super::literal_spelling::safe_edit(
                        file,
                        cx,
                        Correction {
                            range: at..at + 1,
                            replacement: String::new(),
                        },
                    )
                } else {
                    None
                },
            });
        }
    }
}

struct Scanner<'a> {
    file: &'a SourceFile,
    classes: Vec<usize>,
    modes: Vec<bool>,
    extended: bool,
    comment: bool,
    group_comment: bool,
    // Capture headers and named references/calls contain opaque names, not
    // set/group/comment syntax. Retain the terminator across static fragments.
    name_end: Option<u8>,
    // Conservatively retain commas after an unescaped opening brace: deleting
    // one can turn literal `{1\,2}` text into an interval quantifier.
    brace: bool,
    escapes: Vec<(usize, bool)>,
}
impl<'a> Scanner<'a> {
    fn new(file: &'a SourceFile, extended: bool) -> Self {
        Self {
            file,
            classes: vec![],
            modes: vec![],
            extended,
            comment: false,
            group_comment: false,
            name_end: None,
            brace: false,
            escapes: vec![],
        }
    }
    // One forward pass over direct static fragments; interpolation expressions
    // are visited separately by the main walker. No reparses, recursion or
    // rescans proportional to the number of escapes/sets/groups.
    fn scan(&mut self, range: Range<usize>, open: u8, close: u8) {
        let bytes = &self.file.bytes;
        let mut i = range.start;
        while i < range.end {
            let b = bytes[i];
            if let Some(end) = self.name_end {
                if b == end {
                    self.name_end = None;
                }
                i += self
                    .file
                    .encoded_character_width(&bytes[i..range.end])
                    .max(1);
                continue;
            }
            if self.comment {
                self.comment = b != b'\n';
                i += 1;
                continue;
            }
            if self.group_comment {
                if b == b')' {
                    self.group_comment = false;
                    i += 1;
                } else {
                    if b == b'\\' {
                        i += 1;
                    }
                    i += self
                        .file
                        .encoded_character_width(&bytes[i..range.end])
                        .max(1);
                }
                continue;
            }
            if b == b'\\' {
                if self.classes.is_empty()
                    && matches!(bytes.get(i + 1), Some(b'k' | b'g'))
                    && let Some(&delimiter @ (b'<' | b'\'')) = bytes.get(i + 2)
                    && i + 2 < range.end
                {
                    self.name_end = Some(if delimiter == b'<' { b'>' } else { b'\'' });
                    i += 3;
                    continue;
                }
                let at = i;
                let end =
                    super::redundant_regexp_character_class::escape_end(self.file, i, range.end);
                i = end;
                if at + 1 >= range.end {
                    continue;
                }
                let c = bytes[at + 1];
                let width = self
                    .file
                    .encoded_character_width(&bytes[at + 1..range.end])
                    .max(1);
                let escaped = &bytes[at + 1..(at + 1 + width).min(range.end)];
                let in_class = !self.classes.is_empty();
                let edge_hyphen = bytes.get(at + 2) == Some(&b']')
                    || self.classes.last().is_some_and(|&start| start + 1 == at);
                if self.file.character_alphanumeric(escaped)
                    || b" \n[]^\\#".contains(&c)
                    || c == open
                    || c == close
                    || matches!(c, b'@' | b'$') && at > range.start && bytes[at - 1] == b'#'
                    || if in_class {
                        c == b'-' && !edge_hyphen
                    } else {
                        b".*+?{}()|$".contains(&c)
                    }
                {
                    continue;
                }
                // Removing escapes must not create set intersection or POSIX
                // syntax. Keep these offenses visible but retain the escape.
                // Whitespace/CRLF and unknown composite tokens lack a proof.
                let safe = end == at + 1 + width
                    && !c.is_ascii_whitespace()
                    && c != 0x0b
                    && !(c == b'{' && at > range.start && bytes[at - 1] == b'#')
                    && !(c == b',' && !in_class && self.brace)
                    && !(in_class
                        && (c == b'&'
                            // A closing escaped colon can activate [:name:]
                            // too, including when both colons are corrected.
                            || c == b':' && self.classes.len() > 1
                            || b".=".contains(&c)
                                && self.classes.len() > 1
                                && bytes.get(end) != Some(&b']')
                                && self.classes.last().is_some_and(|&start| start + 1 == at)));
                self.escapes.push((at, safe));
                continue;
            }
            if b == b'[' {
                self.classes.push(i);
                i += 1;
                if bytes.get(i) == Some(&b'^') {
                    i += 1;
                }
                // A leading ']' is a literal element, not the set's end.
                if bytes.get(i) == Some(&b']') {
                    i += 1;
                }
                continue;
            }
            if !self.classes.is_empty() {
                if b == b']' {
                    self.classes.pop();
                }
            } else if b == b'{' {
                self.brace = true;
            } else if b == b'}' {
                self.brace = false;
            } else if b == b'#' && self.extended {
                self.comment = true;
            } else if b == b'(' {
                if bytes[i..range.end].starts_with(b"(?#") {
                    self.group_comment = true;
                    i += 3;
                    continue;
                }
                let old = self.extended;
                let tail = &bytes[i..range.end];
                let name_end =
                    if tail.starts_with(b"(?<") && !matches!(tail.get(3), Some(b'=' | b'!')) {
                        Some(b'>')
                    } else if tail.starts_with(b"(?'") {
                        Some(b'\'')
                    } else {
                        None
                    };
                if let Some(end) = name_end {
                    self.modes.push(old);
                    self.name_end = Some(end);
                    i += 3;
                    continue;
                }
                if bytes.get(i + 1) == Some(&b'?') {
                    let mut end = i + 2;
                    let mut enabled = true;
                    while end < range.end && b"imxadu-".contains(&bytes[end]) {
                        match bytes[end] {
                            b'-' => enabled = false,
                            b'x' => self.extended = enabled,
                            _ => {}
                        }
                        end += 1;
                    }
                    if end > i + 2 {
                        if bytes.get(end) == Some(&b')') {
                            i = end + 1;
                            continue;
                        }
                        if bytes.get(end) == Some(&b':') {
                            self.modes.push(old);
                            i = end + 1;
                            continue;
                        }
                    }
                    self.extended = old;
                }
                self.modes.push(old);
            } else if b == b')'
                && let Some(old) = self.modes.pop()
            {
                self.extended = old;
            }
            i += self
                .file
                .encoded_character_width(&bytes[i..range.end])
                .max(1);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop redundant_regexp_escape_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov), with semantic safety regressions.
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantRegexpEscape")
            .collect()
    }
    #[test]
    fn exact_unicode_and_multiline_diagnostics() {
        let found = offenses("p /[一二三四\\.]/\n%r{\n  \\/a\n}x\n");
        assert_eq!(
            found
                .iter()
                .map(|o| (o.line, o.col, o.end_line))
                .collect::<Vec<_>>(),
            [(1, 9, 1), (3, 3, 3)]
        );
        assert!(
            found
                .iter()
                .all(|o| o.message == "Redundant escape inside regexp literal")
        );
        assert_correction(&RedundantRegexpEscape, r"/[\§]/", "/[§]/");
        assert_correction(&RedundantRegexpEscape, r"/a\,b/", "/a,b/");
        assert_correction(&RedundantRegexpEscape, r"/a\{1\,2\}/", r"/a\{1,2\}/");
    }
    #[test]
    fn character_class_edges_nesting_and_operators() {
        for (before, after) in [
            (r"/[\-a]/", "/[-a]/"),
            (r"/[a\-]/", "/[a-]/"),
            (r"/[a-w&&[^c-g]\-1-9]/", r"/[a-w&&[^c-g]\-1-9]/"),
            (r"/[[:punct:]&&[^\.]]/", "/[[:punct:]&&[^.]]/"),
            (r"/[\[\-\]]/", r"/[\[\-\]]/"),
            (r"/\[\+a[a]\+/", r"/\[\+a[a]\+/"),
        ] {
            if before == after {
                assert!(offenses(before).is_empty(), "{before}");
            } else {
                assert_correction(&RedundantRegexpEscape, before, after);
            }
        }
    }
    #[test]
    fn scoped_and_unscoped_options_and_comments() {
        for (before, after) in [
            (
                "/(?x:a # [\\.]\n\\-)(?-x:#\\-)/",
                "/(?x:a # [\\.]\n-)(?-x:#-)/",
            ),
            ("/(?ax:a # [\\.]\n\\-)/", "/(?ax:a # [\\.]\n-)/"),
            ("/(?x)a # \\-\n\\-\\/b/", "/(?x)a # \\-\n-\\/b/"),
            (r"/(?#\) [\.] \-)\-/", r"/(?#\) [\.] \-)-/"),
            (r"/(?x:[#\+])\-/", r"/(?x:[#+])-/"),
        ] {
            assert_correction(&RedundantRegexpEscape, before, after);
        }
    }
    #[test]
    fn composite_escapes_backreferences_and_escaped_backslashes() {
        for s in [
            r"/\M-\C-a/n",
            r"/[\M-\C-a]/n",
            r"/\c\?/",
            r"/\x2d\u002d\u{002d}\p{Alpha}/",
            r"/(?<name>[a-z])\k<name>\g<name>/",
            r"/(a)\1/",
            r"/\\-\\[\\+]/",
            r"/\é\一/",
            "/a\\\nb/",
            "/[a\\\nb]/",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn named_tokens_are_not_regexp_structure() {
        for s in [
            r"/(?<a[b>x)\+/",
            r"/(?'a[b'x)\+/",
            r"/(?<a[b>x)\k<a[b>\+/",
            r"/(?'a[b'x)\k'a[b'\+/",
            r"/(?<a[b>x)\g<a[b>\+/",
            r"/(?'a[b'x)\g'a[b'\+/",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        // Closing brackets and comment/option punctuation in names must not
        // hide real escapes in the following body or change the mode stack.
        assert_correction(
            &RedundantRegexpEscape,
            r"/(?<a]b>[\+])\k<a]b>\-/",
            r"/(?<a]b>[+])\k<a]b>-/",
        );
        for s in [r"/(?<=x)\+/", r"/(?<!x)\+/"] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        assert_correction(
            &RedundantRegexpEscape,
            r"/(?<a[b>[\+])\k<a[b>\-/",
            r"/(?<a[b>[+])\k<a[b>-/",
        );
        assert_correction(
            &RedundantRegexpEscape,
            r"/(?x:(?<a#(b>x))\-/",
            r"/(?x:(?<a#(b>x))-/",
        );
    }
    #[test]
    fn delimiters_options_and_match_last_line() {
        for (open, close) in [
            ("/", "/"),
            ("%r/", "/"),
            ("%r{", "}"),
            ("%r!", "!"),
            ("%r<", ">"),
            ("%r(", ")"),
            ("%r'", "'"),
        ] {
            for options in ["", "i", "mx", "o", "n", "u", "e", "s"] {
                let before = format!("{open}a\\-b{close}{options}");
                let after = format!("{open}a-b{close}{options}");
                assert_correction(&RedundantRegexpEscape, &before, &after);
            }
        }
        assert_correction(
            &RedundantRegexpEscape,
            "p :yes if /\\-/\n",
            "p :yes if /\\-/\n",
        );
        assert_correction(
            &RedundantRegexpEscape,
            "p :yes if /a#{/\\-/}c/\n",
            "p :yes if /a#{/\\-/}c/\n",
        );
    }
    #[test]
    fn closed_literal_match_predicates_keep_safe_fixes() {
        assert_correction(
            &RedundantRegexpEscape,
            r"p 'a-b'.match?(/a\-b/)",
            "p 'a-b'.match?(/a-b/)",
        );
        for s in [
            r"unknown.match?(/a\-b/)",
            r"effect; 'a'.match?(/a\-b/)",
            r"def f; 'a'.match?(/a\-b/); end",
        ] {
            let found = correction_offenses(&RedundantRegexpEscape, s);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn interpolation_code_is_not_outer_regexp_text() {
        assert!(offenses(r##"/#{"\\-"}/"##).is_empty());
        for s in [
            r"/[#\@ivar]/",
            r"/[#\@@cvar]/",
            r"/[#\$gvar]/",
            r"/%r\#\{x\}/",
            r"/\#\{[a-z_]+\}/",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for s in [
            r"/#{x}[\+]/",
            r"/[#{x}\+]/",
            r"/#{x}\-]x/",
            r"/#@name\_/",
            r"/#$name\_/",
            r"/#@@name\_/",
        ] {
            let found = correction_offenses(&RedundantRegexpEscape, s);
            assert!(!found.is_empty(), "{s}");
            assert!(
                found.iter().all(|o| o.correction.is_none()),
                "{s}: {found:?}"
            );
        }
    }
    #[test]
    fn set_operator_posix_quantifier_and_interpolation_activation_edits_withheld() {
        for s in [
            r"/[a\&&b]/",
            r"/[a\&\&b]/",
            r"/[[\:digit:]]/",
            r"/[[:digit\:]]/",
            r"/[[\:digit\:]]/",
            r"/[a&&[:digit\:]]/",
            r"/[#\{name}]/",
            r"/a{1\,2}/",
            r"/a{\,2}/",
            r"/a{1\,}/",
            "/[a\\\r\n]/",
        ] {
            let found = correction_offenses(&RedundantRegexpEscape, s);
            assert!(!found.is_empty(), "{s}");
            assert!(
                found.iter().all(|o| o.correction.is_none()),
                "{s}: {found:?}"
            );
        }
    }
    #[test]
    fn wide_and_deep_scans_are_iterative_and_bounded() {
        let wide = format!("/{}{}\\n/", "[\\+]".repeat(6000), "\\-".repeat(6000));
        assert_eq!(offenses(&wide).len(), 12000);
        let deep = format!("{}\\+{}", "[".repeat(6000), "]".repeat(6000));
        let file = SourceFile::new("a.rb".into(), deep.into_bytes());
        let mut scanner = Scanner::new(&file, false);
        scanner.scan(0..file.bytes.len(), b'/', b'/');
        assert_eq!(scanner.escapes.len(), 1);
        assert!(scanner.classes.is_empty());
        let opaque = format!("/{}{}\\-]/", "[".repeat(6000), "#{x}".repeat(6000));
        let found = correction_offenses(&RedundantRegexpEscape, &opaque);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }
}
