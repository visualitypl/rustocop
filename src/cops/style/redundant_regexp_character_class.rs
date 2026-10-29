//! `Style/RedundantRegexpCharacterClass` — single-element regexp sets.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
use std::ops::Range;
pub struct RedundantRegexpCharacterClass;
impl Cop for RedundantRegexpCharacterClass {
    fn name(&self) -> &'static str {
        "Style/RedundantRegexpCharacterClass"
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
        let mut scanner = Scanner {
            file,
            classes: vec![],
            modes: vec![],
            extended: regexp.closing.as_slice().contains(&b'x'),
            comment: false,
            group_comment: false,
            ranges: vec![],
            epoch: 0,
        };
        let mut previous = regexp.opening.end_offset();
        let interpolated = matches!(
            node,
            Node::InterpolatedRegularExpressionNode { .. }
                | Node::InterpolatedMatchLastLineNode { .. }
        );
        for fragment in regexp.fragments {
            if fragment.start_offset() != previous {
                // Interpolation is opaque regex text. A spanning class cannot
                // be proved a singleton; still inspect subsequent complete sets.
                scanner.epoch += 1;
            }
            scanner.scan(fragment.start_offset()..fragment.end_offset());
            previous = fragment.end_offset();
        }
        let unobserved = cx.corrections_requested()
            && !scanner.ranges.is_empty()
            && super::exact_regexp_match::spelling_value_is_unobserved(node, cx, file);
        for (range, safe_context) in scanner.ranges {
            let raw = &file.bytes[range.clone()];
            let element = if raw == b"[#]" {
                b"\\#".as_slice()
            } else {
                &raw[1..raw.len() - 1]
            };
            let (line, col) = file.line_col(range.start);
            let correction = if unobserved && !interpolated && safe_context && safe_element(element)
            {
                std::str::from_utf8(element).ok().and_then(|text| {
                    super::literal_spelling::safe_edit(
                        file,
                        cx,
                        Correction {
                            range: range.clone(),
                            replacement: text.into(),
                        },
                    )
                })
            } else {
                None
            };
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "Redundant single-element character class, `{}` can be replaced with `{}`.",
                    String::from_utf8_lossy(raw),
                    String::from_utf8_lossy(element)
                ),
                line,
                col,
                end_line: file.line_of(range.end - 1),
                correction,
            });
        }
    }
}
struct Class {
    start: usize,
    epoch: usize,
    count: usize,
    // Nested/negative sets, ranges and intersections are never single elements.
    excluded: bool,
    element: Range<usize>,
    safe_context: bool,
}
struct Scanner<'a> {
    file: &'a SourceFile,
    classes: Vec<Class>,
    modes: Vec<bool>,
    extended: bool,
    comment: bool,
    group_comment: bool,
    ranges: Vec<(Range<usize>, bool)>,
    epoch: usize,
}
impl Scanner<'_> {
    // Iterative and linear, including nested sets, scoped options and opaque
    // interpolation gaps. Never parse a regexp/file again inside a cop.
    fn scan(&mut self, range: Range<usize>) {
        let bytes = &self.file.bytes;
        let mut i = range.start;
        let mut short_hex_end = None;
        while i < range.end {
            let b = bytes[i];
            if self.group_comment {
                if b == b')' {
                    self.group_comment = false;
                    i += 1;
                } else {
                    // Only an unescaped ')' ends a group comment. Skip whole
                    // encoded characters, including escaped multibyte text.
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
            if self.comment {
                self.comment = b != b'\n';
                i += 1;
                continue;
            }
            if b == b'\\' {
                let end = escape_end(self.file, i, range.end);
                // A short hexadecimal escape can absorb a following a-f once
                // brackets disappear. Keep this token boundary, not a raw-byte
                // lookbehind (which would mistake escaped backslashes for it).
                short_hex_end = (bytes.get(i + 1) == Some(&b'x') && end == i + 3).then_some(end);
                if let Some(class) = self.classes.last_mut() {
                    class.count += 1;
                    class.element = i..end;
                }
                i = end;
                continue;
            }
            if b == b'[' {
                // Nested bracket removal can expose '-'/'&&' operators, even
                // when the element itself is a letter. Keep reporting these
                // singletons, but retain their set grouping conservatively.
                let safe_context = self.classes.is_empty() && short_hex_end != Some(i);
                if let Some(parent) = self.classes.last_mut() {
                    parent.count += 1;
                    parent.excluded = true;
                }
                self.classes.push(Class {
                    start: i,
                    epoch: self.epoch,
                    count: 0,
                    excluded: false,
                    element: i..i,
                    safe_context,
                });
                i += 1;
                if bytes.get(i) == Some(&b'^') {
                    self.classes.last_mut().unwrap().excluded = true;
                    i += 1;
                }
                // ] is a literal when it is the first set element.
                if bytes.get(i) == Some(&b']') {
                    let class = self.classes.last_mut().unwrap();
                    class.count = 1;
                    class.element = i..i + 1;
                    class.excluded = true;
                    i += 1;
                }
                continue;
            }
            if let Some(class) = self.classes.last_mut() {
                if b == b']' {
                    let class = self.classes.pop().unwrap();
                    let elem = &bytes[class.element.clone()];
                    if class.epoch == self.epoch
                        && class.count == 1
                        && !class.excluded
                        && redundant(elem, self.extended)
                    {
                        self.ranges.push((class.start..i + 1, class.safe_context));
                    }
                    i += 1;
                    continue;
                }
                let width = self
                    .file
                    .encoded_character_width(&bytes[i..range.end])
                    .max(1);
                class.count += 1;
                class.element = i..i + width;
                i += width;
                continue;
            }
            if b == b'#' && self.extended {
                self.comment = true;
                i += 1;
                continue;
            }
            if b == b'(' {
                if bytes[i..range.end].starts_with(b"(?#") {
                    self.group_comment = true;
                    i += 3;
                    continue;
                }
                let old = self.extended;
                let mut end = i + 2;
                let mut enabled = true;
                if bytes.get(i + 1) == Some(&b'?') {
                    while end < range.end && b"imx-".contains(&bytes[end]) {
                        match bytes[end] {
                            b'-' => enabled = false,
                            b'x' => self.extended = enabled,
                            _ => {}
                        }
                        end += 1;
                    }
                    if end > i + 2 && bytes.get(end) == Some(&b')') {
                        i = end + 1;
                        continue;
                    }
                    if end > i + 2 && bytes.get(end) == Some(&b':') {
                        self.modes.push(old);
                        i = end + 1;
                        continue;
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
pub(super) fn escape_end(file: &SourceFile, mut start: usize, limit: usize) -> usize {
    let bytes = &file.bytes;
    // Control/meta prefixes can be combined (\\M-\\C-a). Consume the chain
    // iteratively instead of recursing in proportion to untrusted pattern text.
    loop {
        if start + 1 >= limit {
            return limit;
        }
        let next = match bytes[start + 1] {
            b'c' => Some(start + 2),
            b'C' | b'M' if bytes.get(start + 2) == Some(&b'-') => Some(start + 3),
            _ => None,
        };
        let Some(next) = next else { break };
        if next >= limit {
            return limit;
        }
        if bytes[next] == b'\\' {
            start = next;
            continue;
        }
        return (next + file.encoded_character_width(&bytes[next..limit]).max(1)).min(limit);
    }
    let kind = bytes[start + 1];
    let mut end = start
        + 1
        + file
            .encoded_character_width(&bytes[start + 1..limit])
            .max(1);
    match kind {
        b'u' | b'p' | b'P' if bytes.get(end) == Some(&b'{') => {
            end += 1;
            while end < limit && bytes[end] != b'}' {
                end += 1;
            }
            end = (end + 1).min(limit);
        }
        b'u' => {
            for _ in 0..4 {
                if end < limit && bytes[end].is_ascii_hexdigit() {
                    end += 1;
                }
            }
        }
        b'x' => {
            for _ in 0..2 {
                if end < limit && bytes[end].is_ascii_hexdigit() {
                    end += 1;
                }
            }
        }
        b'0'..=b'7' => {
            for _ in 0..2 {
                if end < limit && matches!(bytes[end], b'0'..=b'7') {
                    end += 1;
                }
            }
        }
        _ => {}
    }
    end
}
// Removing brackets can fuse adjacent numeric escapes/backreferences. Numeric
// elements stay reported but unedited, as do context-sensitive/unknown escapes.
fn safe_element(elem: &[u8]) -> bool {
    if elem.first().is_some_and(u8::is_ascii_digit) {
        return false;
    }
    if !elem.starts_with(b"\\") {
        return true;
    }
    match elem.get(1) {
        Some(b'0') => elem.len() == 4,
        Some(b'x') => elem.len() == 4,
        Some(b'u') => elem.len() == 6 || elem.get(2) == Some(&b'{'),
        Some(b'p' | b'P') => elem.get(2) == Some(&b'{'),
        Some(
            b's' | b'S' | b'd' | b'D' | b'w' | b'W' | b'h' | b'H' | b'a' | b'e' | b'f' | b'n'
            | b'r' | b't' | b'v',
        ) => elem.len() == 2,
        Some(b) => elem.len() == 2 && b.is_ascii_punctuation(),
        None => false,
    }
}
fn redundant(elem: &[u8], extended: bool) -> bool {
    if elem == b"\\b"
        || elem.len() == 2 && elem[0] == b'\\' && matches!(elem[1], b'1'..=b'9')
        || elem.len() == 1 && b".*+?{}()|$".contains(&elem[0])
        || extended && elem.iter().any(|b| b.is_ascii_whitespace() || *b == 0x0b)
    {
        return false;
    }
    if elem.starts_with(b"\\u{") {
        return elem[3..elem.len() - 1]
            .split(u8::is_ascii_whitespace)
            .filter(|x| !x.is_empty())
            .count()
            == 1;
    }
    true
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop redundant_regexp_character_class_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::assert_correction;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantRegexpCharacterClass")
            .collect()
    }
    #[test]
    fn pinned_singletons_exact_diagnostics_and_corrections() {
        for element in [
            "a",
            "é",
            r"\]",
            r"\[",
            r"\s",
            r"\S",
            r"\u{06F2}",
            r"\p{Digit}",
            r"\P{Digit}",
            r"\#",
            r"\032",
            r"\+",
            " ",
        ] {
            for (open, close) in [("/", "/"), ("%r{", "}")] {
                let s = format!("foo = {open}[{element}]{close}");
                let found = offenses(&s);
                assert_eq!(found.len(), 1, "{s}");
                assert_eq!((found[0].line, found[0].col), (1, 7 + open.len() as u32));
                assert_eq!(
                    found[0].message,
                    format!(
                        "Redundant single-element character class, `[{element}]` can be replaced with `{element}`."
                    )
                );
                assert_correction(&RedundantRegexpCharacterClass, &s, &s);
                assert_correction(
                    &RedundantRegexpCharacterClass,
                    &format!("{open}[{element}]{close}"),
                    &format!("{open}{element}{close}"),
                );
            }
        }
        assert_correction(&RedundantRegexpCharacterClass, r"/[#]{0}/", r"/\#{0}/");
    }
    #[test]
    fn pinned_multiple_groups_quantifiers_and_interpolations() {
        for (before, after, count) in [
            ("/[a]b[c]d/", "/abcd/", 2),
            ("/([a])+/", "/(a)+/", 1),
            ("/[a]{2,10}/", "/a{2,10}/", 1),
            ("/a#{/[b]/}c/", "/a#{/[b]/}c/", 1),
            ("/#{x}[a]/", "/#{x}[a]/", 1),
            ("/a # [b]\n[c]/x", "/a # [b]\nc/x", 1),
        ] {
            assert_eq!(offenses(before).len(), count, "{before}");
            assert_correction(&RedundantRegexpCharacterClass, before, after);
        }
    }
    #[test]
    fn closed_literal_match_predicates_keep_safe_fixes() {
        assert_correction(
            &RedundantRegexpCharacterClass,
            "p 'a'.match?(/[a]/)",
            "p 'a'.match?(/a/)",
        );
        for s in [
            "unknown.match?(/[a]/)",
            "effect; 'a'.match?(/[a]/)",
            "def f; 'a'.match?(/[a]/); end",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&RedundantRegexpCharacterClass, s);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn pinned_negative_lookalikes_backreferences_and_options() {
        assert!(offenses("/[\u{000b}]/x").is_empty());
        for s in [
            r"/[a-z]/",
            r"/[[:alnum:]]/",
            r"/[[:^alnum:]]/",
            r"/[[:alnum:]&&a-d]/",
            r"/[\u{0061 0062}]/",
            r"/[\b]/",
            r"/[\1]/",
            r"/[\8]/",
            r"/[\9]/",
            r"/[+]/",
            r"/[\])]/",
            r"/[^x]/",
            r"/[#{foo}]/",
            r"/\[\]/",
            r"/[\[\]]/",
            r"/\[x\]/",
            r"/[ab]/",
            r"/a#{b[0]}c/",
            r"/foo # [a]/x",
            r"/[ ]/x",
            r"/[ ]/ix",
            r"/[ ]/iux",
            r"/(?x:[ ])/",
            r"/(?# [a])b/",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn escaped_multibyte_characters_and_combined_control_tokens() {
        for (s, elem) in [
            (r"/[\é]/", r"\é"),
            (r"/[\M-\C-a]/n", r"\M-\C-a"),
            (r"/[[\M-\C-a]]/n", r"\M-\C-a"),
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!(
                found[0].message,
                format!(
                    "Redundant single-element character class, `[{elem}]` can be replaced with `{elem}`."
                )
            );
        }
    }
    #[test]
    fn lexical_fusion_context_sensitive_escapes_and_opaque_interpolation_withhold() {
        for s in [
            r"/(a)\1[2]/",
            r"/[\0]1/",
            r"/[\x1]2/",
            r"/[\B]/",
            r"/#{x}[a]/",
            r"/[\0][1]/",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&RedundantRegexpCharacterClass, s);
            assert!(!found.is_empty(), "{s}");
            assert!(
                found.iter().all(|o| o.correction.is_none()),
                "{s}: {found:?}"
            );
        }
    }
    #[test]
    fn preceding_short_hex_escape_withholds_without_hiding_offenses() {
        for s in [r"/\x1[a]/", r"/\xA[F]/", r"/\x1[a][b]/"] {
            let found =
                crate::cops::testing::correction_offenses(&RedundantRegexpCharacterClass, s);
            assert!(!found.is_empty(), "{s}");
            assert!(found[0].correction.is_none(), "{s}: {found:?}");
        }
        for (before, after) in [(r"/\x01[a]/", r"/\x01a/"), (r"/\\x1[a]/", r"/\\x1a/")] {
            assert_correction(&RedundantRegexpCharacterClass, before, after);
        }
    }
    #[test]
    fn escaped_group_comment_terminators_preserve_option_state() {
        for s in [r"/(?#\) [a])/", r"/(?#\) (?-x:(?-x:\) )[ ]/x"] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        assert_correction(
            &RedundantRegexpCharacterClass,
            r"/(?#\) [a])[b]/",
            r"/(?#\) [a])b/",
        );
        assert_correction(&RedundantRegexpCharacterClass, r"/(?#\\)[a]/", r"/(?#\\)a/");
    }
    #[test]
    fn nested_singletons_withhold_set_operator_fusion() {
        for s in [r"/[a[-]z]/", r"/[a[&]&b]/", r"/[[a]-z]/"] {
            let found =
                crate::cops::testing::correction_offenses(&RedundantRegexpCharacterClass, s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none(), "{s}: {found:?}");
        }
        assert_correction(&RedundantRegexpCharacterClass, "/[-][&]/", "/-&/");
    }
    #[test]
    fn wide_and_deep_sets_and_interpolation_gaps_are_bounded() {
        let wide = format!("/{}\\n/", "[a]".repeat(6000));
        assert_eq!(offenses(&wide).len(), 6000);
        let deep = format!("/{}a{}/", "[".repeat(6000), "]".repeat(6000));
        // Prism rejects extremely deep static sets before cop dispatch. Exercise
        // our iterative scanner directly too (no Ruby/parser recursion involved).
        let file = SourceFile::new("a.rb".into(), deep.into_bytes());
        let mut scanner = Scanner {
            file: &file,
            classes: vec![],
            modes: vec![],
            extended: false,
            comment: false,
            group_comment: false,
            ranges: vec![],
            epoch: 0,
        };
        scanner.scan(1..file.bytes.len() - 1);
        assert_eq!(scanner.ranges.len(), 1);
        let opaque = format!(
            "/{}{}a{}/",
            "[".repeat(6000),
            "#{x}".repeat(6000),
            "]".repeat(6000)
        );
        assert!(offenses(&opaque).is_empty());
    }
    #[test]
    fn inline_extended_modes_and_nested_set_positions() {
        for (s, expected) in [
            ("/(?x:[a] # [b]\n)[c]/", 2),
            ("/(?x:[ ])(?-x:[ ])/", 1),
            ("/(?x)(?-x:[ ])[a]/", 2),
            ("/[[a]b]/", 1),
            ("/[x&&[a]]/", 1),
        ] {
            assert_eq!(offenses(s).len(), expected, "{s}");
        }
    }
}
