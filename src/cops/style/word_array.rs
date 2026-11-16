//! `Style/WordArray` — fixed percent style, MinSize 2.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{ArrayNode, Node, StringNode};
pub struct WordArray;
impl Cop for WordArray {
    fn name(&self) -> &'static str {
        "Style/WordArray"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_array_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(array) = node.as_array_node() else {
            return;
        };
        // Parent-first dispatch: each matrix inspects its own direct rows once.
        // Children only query the recorded offset, never rescan siblings/root.
        if array.elements().iter().all(|n| n.as_array_node().is_some())
            && array.elements().iter().any(|n| {
                let row = n.as_array_node().unwrap();
                let lower = row
                    .opening_loc()
                    .is_some_and(|l| l.as_slice().starts_with(b"%w"));
                row.elements()
                    .iter()
                    .any(|s| complex(&s, file, true, lower))
            })
        {
            file.word_array_matrices()
                .lock()
                .unwrap()
                .insert(array.location().start_offset());
        }
        let Some(open) = array.opening_loc() else {
            return;
        };
        let percent = open.as_slice().starts_with(b"%w") || open.as_slice().starts_with(b"%W");
        if percent {
            if !array
                .elements()
                .iter()
                .any(|s| complex(&s, file, false, open.as_slice().starts_with(b"%w")))
            {
                return;
            }
        } else {
            if open.as_slice() != b"["
                || array.elements().len() < 2
                || array.elements().iter().any(|n| {
                    n.as_string_node().is_none_or(|s| {
                        !reference_static(&s, file, false)
                            || s.opening_loc()
                                .is_some_and(|l| l.as_slice().starts_with(b"<<"))
                    }) || complex(&n, file, true, false)
                })
            {
                return;
            }
            if cx.parent().is_some_and(|p| {
                p.as_array_node().is_some()
                    && file
                        .word_array_matrices()
                        .lock()
                        .unwrap()
                        .contains(&p.location().start_offset())
            }) {
                return;
            }
            if super::correction::contains_comment(
                file,
                array.location().start_offset()..array.location().end_offset(),
            ) || ambiguous_context(cx)
            {
                return;
            }
        }
        let replacement = if percent {
            brackets(&array, file)
        } else if cx.corrections_requested() {
            words(&array)
        } else {
            None
        };
        let message = if percent {
            match &replacement {
                Some(s) if !s.contains('\n') => format!("Use `{s}` for an array of words."),
                _ => "Use an array literal `[...]` for an array of words.".into(),
            }
        } else {
            "Use `%w` or `%W` for an array of words.".into()
        };
        let loc = array.location();
        let (line, col) = file.line_col(loc.start_offset());
        let correction = if cx.corrections_requested() {
            replacement.and_then(|replacement| {
                // No transcoding: non-UTF8 interiors/forced binary literals stay
                // reported. Dynamic percent words retain all interpolation code.
                if !file.is_utf8()
                    && array.elements().iter().any(|n| {
                        n.as_string_node()
                            .is_some_and(|s| !s.unescaped().is_ascii())
                    })
                {
                    return None;
                }
                Some(Correction {
                    range: loc.start_offset()..loc.end_offset(),
                    replacement,
                })
            })
        } else {
            None
        };
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.last_line_of(loc),
            correction,
        });
    }
}
pub(super) fn ambiguous_context(cx: &NodeContext<'_, '_>) -> bool {
    // Only an unparenthesized argument risks rebinding the following block;
    // a receiver can always use percent-array syntax.
    if cx.parent().is_none_or(|p| p.as_arguments_node().is_none()) {
        return false;
    }
    let Some(parent) = cx.ancestors().find(|p| p.as_arguments_node().is_none()) else {
        return false;
    };
    parent.as_call_node().is_some_and(|c| {
        c.opening_loc().is_none() && c.block().is_some_and(|b| b.as_block_node().is_some())
    })
}
fn complex(node: &Node<'_>, file: &SourceFile, word: bool, lower: bool) -> bool {
    let Some(s) = node.as_string_node() else {
        return false;
    };
    #[cfg(test)]
    CLASSIFIED_BYTES
        .set(CLASSIFIED_BYTES.get() + s.content_loc().as_slice().len() + s.unescaped().len());
    if !reference_static(&s, file, lower) {
        return false;
    }
    let Some(bytes) = projected(&s, file, lower) else {
        return true;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return true;
    };
    if text.contains(' ') {
        return true;
    }
    if !word {
        return false;
    }
    let chars: Vec<_> = text.chars().collect();
    let is_word = |c: char| {
        let mut b = [0; 4];
        file.word_character(c.encode_utf8(&mut b).as_bytes(), true)
    };
    // The fixed regex is (Word | Word-Word | LF | TAB)+, not a general
    // hyphenated-word grammar: a-b-c is invalid, ab-cd is valid.
    let mut i = 0;
    while i < chars.len() {
        if is_word(chars[i]) && i + 2 < chars.len() && chars[i + 1] == '-' && is_word(chars[i + 2])
        {
            i += 3;
        } else if is_word(chars[i]) || matches!(chars[i], '\n' | '\t') {
            i += 1;
        } else {
            return true;
        }
    }
    chars.is_empty()
}
// The pinned Prism-to-RuboCop builder represents empty percent strings and
// physically split static strings as dstr, not str. Continuations in double
// quotes produce no fragment. Bounded by this string's own content.
fn reference_static(s: &StringNode<'_>, file: &SourceFile, lower: bool) -> bool {
    let loc = s.content_loc();
    let opening = s.opening_loc();
    // Percent-array words are already single static Ruby strings, even when
    // escaped physical whitespace splits their token across lines.
    if opening.is_none() {
        return true;
    }
    if loc.as_slice().is_empty()
        && opening
            .as_ref()
            .is_some_and(|l| l.as_slice().starts_with(b"%"))
    {
        return false;
    }
    let single =
        lower || opening.is_some_and(|l| l.as_slice() == b"'" || l.as_slice().starts_with(b"%q"));
    let mut heads = file
        .character_offsets(loc.start_offset()..loc.end_offset())
        .peekable();
    while let Some(i) = heads.next() {
        if !single && file.bytes[i] == b'\\' {
            if let Some(j) = heads.next()
                && file.bytes[j] == b'\r'
                && heads.peek().is_some_and(|&k| file.bytes[k] == b'\n')
            {
                heads.next();
            }
        } else if file.bytes[i] == b'\n' && i + 1 < loc.end_offset() {
            return false;
        }
    }
    true
}

/// Project only raw legacy characters, not bytes introduced by escapes. The
/// main Prism payload remains the authority for every decoded escape. Bounded
/// source escape units synchronize raw and decoded cursors; no parse/codec.
fn projected(s: &StringNode<'_>, file: &SourceFile, lower: bool) -> Option<Vec<u8>> {
    if !file.word_projection_required() || s.is_forced_utf8_encoding() {
        return Some(s.unescaped().to_vec());
    }
    let loc = s.content_loc();
    let raw = loc.as_slice();
    let decoded = s.unescaped();
    let opening = s.opening_loc();
    let single =
        lower || opening.is_some_and(|l| l.as_slice() == b"'" || l.as_slice().starts_with(b"%q"));
    let mut out = Vec::new();
    let mut r = 0;
    let mut d = 0;
    while r < raw.len() {
        // The reference buffer projects raw characters BEFORE Ruby escapes:
        // UTF8-MAC composition, CESU surrogate pairs and carrier keycaps can
        // change Word-unit counts, without changing the Ruby escape payload.
        if let Some((width, units)) = file.word_prefix(&raw[r..]) {
            if decoded.get(d..d + width)? != &raw[r..r + width] {
                return None;
            }
            if units == 0 {
                out.push(b'!');
            } else {
                out.extend(std::iter::repeat_n(b'a', units));
            }
            r += width;
            d += width;
            continue;
        }
        let absolute = loc.start_offset() + r;
        let width = file
            .character_offsets(absolute..loc.end_offset())
            .nth(1)
            .map_or(raw.len() - r, |next| next - absolute);
        if raw[r] >= 128 {
            let bytes = raw.get(r..r + width)?;
            if decoded.get(d..d + width)? != bytes {
                return None;
            }
            out.push(if file.word_character(bytes, false) {
                b'a'
            } else {
                b'!'
            });
            r += width;
            d += width;
            continue;
        }
        if raw[r] != b'\\' || r + 1 == raw.len() {
            // CRLF is normalized by Ruby in literal data.
            if raw[r] == b'\r' && raw.get(r + 1) == Some(&b'\n') {
                r += 1;
            }
            out.push(*decoded.get(d)?);
            r += 1;
            d += 1;
            continue;
        }
        let next = raw[r + 1];
        if single {
            if decoded.get(d) == Some(&b'\\') && next != b'\\' && decoded.get(d + 1) == Some(&next)
            {
                out.push(b'\\');
                d += 1;
                r += 1;
                continue;
            }
            r += 1; // escaped raw character, including a legacy multibyte head
            if next >= 128 {
                continue;
            }
            out.push(*decoded.get(d)?);
            d += 1;
            r += 1;
            continue;
        }
        if next >= 128 {
            r += 1;
            continue;
        }
        let start = d;
        let mut end = r + 2;
        match next {
            b'\n' => {}
            b'\r' if raw.get(end) == Some(&b'\n') => {
                end += 1;
            }
            b'x' => {
                while end < raw.len() && end < r + 4 && raw[end].is_ascii_hexdigit() {
                    end += 1;
                }
                d += 1;
            }
            b'0'..=b'7' => {
                while end < raw.len() && end < r + 4 && matches!(raw[end], b'0'..=b'7') {
                    end += 1;
                }
                d += 1;
            }
            b'u' => {
                if raw.get(end) == Some(&b'{') {
                    end += 1;
                    let first = end;
                    while raw.get(end) != Some(&b'}') {
                        end += 1;
                        if end >= raw.len() {
                            return None;
                        }
                    }
                    let text = std::str::from_utf8(&raw[first..end]).ok()?;
                    for value in text.split_ascii_whitespace() {
                        let c = char::from_u32(u32::from_str_radix(value, 16).ok()?)?;
                        d += c.len_utf8();
                    }
                    end += 1;
                } else {
                    let text = std::str::from_utf8(raw.get(end..end + 4)?).ok()?;
                    d += char::from_u32(u32::from_str_radix(text, 16).ok()?)?.len_utf8();
                    end += 4;
                }
            }
            b'c' | b'C' | b'M' => {
                // Ruby's control/meta grammar consumes one target, possibly
                // another control/meta escape. All yield one decoded byte.
                if next != b'c' {
                    if raw.get(end) != Some(&b'-') {
                        return None;
                    }
                    end += 1;
                }
                let mut consumed = false;
                while raw.get(end) == Some(&b'\\') {
                    end += 1;
                    let kind = *raw.get(end)?;
                    end += 1;
                    if matches!(kind, b'M' | b'C') {
                        if raw.get(end) != Some(&b'-') {
                            return None;
                        }
                        end += 1;
                    } else if kind != b'c' {
                        let first = end;
                        if kind == b'x' {
                            while end < first + 2 && raw.get(end).is_some_and(u8::is_ascii_hexdigit)
                            {
                                end += 1;
                            }
                        } else if matches!(kind, b'0'..=b'7') {
                            while end < first + 2
                                && raw.get(end).is_some_and(|b| matches!(b, b'0'..=b'7'))
                            {
                                end += 1;
                            }
                        }
                        consumed = true;
                        break;
                    }
                }
                if !consumed {
                    end += 1;
                }
                d += 1;
            }
            _ => {
                d += 1;
            }
        }
        out.extend_from_slice(decoded.get(start..d)?);
        r = end;
    }
    (d == decoded.len()).then_some(out)
}
fn escaped(s: &StringNode<'_>, percent: bool, upper: bool) -> Option<String> {
    if s.is_forced_binary_encoding() {
        return None;
    }
    let text = std::str::from_utf8(s.unescaped()).ok()?;
    let mut out = String::new();
    for (i, c) in text.chars().enumerate() {
        if upper && s.is_forced_utf8_encoding() && i == 0 {
            out.push_str(&format!("\\u{{{:x}}}", c as u32));
            continue;
        }
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '"' if !percent => out.push_str("\\\""),
            '#' if upper => out.push_str("\\#"),
            '[' | ']' | ' ' if percent => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    Some(out)
}
#[cfg(test)]
thread_local! {
    static CLASSIFIED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static WORD_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
// Ruby code separators are not percent-word contents: in particular a code
// continuation must become separator LF, NOT an escaped LF inside a word.
pub(super) fn word_separator(raw: &[u8]) -> Option<String> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            b',' => {}
            b'\\'
                if raw.get(i + 1) == Some(&b'\n')
                    || (raw.get(i + 1) == Some(&b'\r') && raw.get(i + 2) == Some(&b'\n')) => {}
            b if super::is_ruby_whitespace(b) => out.push(b),
            _ => return None,
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}
fn words(array: &ArrayNode<'_>) -> Option<String> {
    #[cfg(test)]
    WORD_BUILDS.set(WORD_BUILDS.get() + 1);
    let elements: Vec<_> = array
        .elements()
        .iter()
        .map(|n| n.as_string_node())
        .collect::<Option<_>>()?;
    let upper = elements.iter().any(|s| {
        s.is_forced_utf8_encoding() || s.unescaped().iter().any(|b| matches!(b, b'\n' | b'\t'))
    });
    let open = array.opening_loc()?;
    let close = array.closing_loc()?;
    let mut out = if upper { "%W[" } else { "%w[" }.to_owned();
    let source = array.location().as_slice();
    let base = array.location().start_offset();
    let mut cursor = open.end_offset();
    for s in elements {
        let gap = word_separator(&source[cursor - base..s.location().start_offset() - base])?;
        if cursor != open.end_offset() && gap.is_empty() {
            out.push(' ');
        } else {
            out.push_str(&gap);
        }
        out.push_str(&escaped(&s, true, upper)?);
        // Preserve physical line counts even for newline-containing/continued
        // element tokens. Separator newlines cannot change a word's value.
        for line in s.location().as_slice().split_inclusive(|&b| b == b'\n') {
            if line.ends_with(b"\n") {
                out.push_str(if line.ends_with(b"\r\n") {
                    "\r\n"
                } else {
                    "\n"
                });
            }
        }
        cursor = s.location().end_offset();
    }
    out.push_str(&word_separator(
        &source[cursor - base..close.start_offset() - base],
    )?);
    out.push(']');
    Some(out)
}
/// RuboCop's `build_bracketed_array`: each word as `to_string_literal`
/// spells it.
fn brackets(array: &ArrayNode<'_>, file: &SourceFile) -> Option<String> {
    let words = array
        .elements()
        .iter()
        .map(|word| {
            if let Some(s) = word.as_string_node() {
                super::correction::to_string_literal(s.unescaped(), file)
            } else {
                let literal =
                    super::correction::to_string_literal(word.location().as_slice(), file)?;
                Some(super::correction::trim_interpolation_escape(&literal))
            }
        })
        .collect::<Option<Vec<_>>>()?;
    super::correction::bracketed_array(array, &words, file)
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop word_array_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, offense::Offense};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == WordArray.name())
            .collect()
    }
    #[test]
    fn utf8_mac_buffer_composition_does_not_invent_extra_word_units_between_hyphens() {
        for word in [
            "a-e\u{301}-c",
            "a-Å\u{301}-c",
            "a-A\u{30a}\u{301}-c",
            "a-\u{1100}\u{1161}-c",
            "a-가\u{11a8}-c",
        ] {
            assert!(
                offenses(&format!("# encoding: UTF8-MAC\n['{word}', 'two']")).is_empty(),
                "{word}"
            );
        }
        assert_eq!(
            offenses("# encoding: UTF8-MAC\n[\"a-e\\u0301-c\",'two']").len(),
            1
        );
        // Report the bracket in ORIGINAL source coordinates. The
        // reference's composed buffer points one character earlier here.
        let found = offenses("# encoding: UTF8-MAC\ns=\"e\u{301}\"; ['one','two']");
        assert_eq!((found[0].line, found[0].col), (2, 9));
    }
    #[test]
    fn exact_ruby_unicode_15_word_class_and_hyphen_grammar() {
        for s in [
            "['\u{301}','two']",
            "['\u{203f}','two']",
            "['a-b','two']",
            "['ab-cd','two']",
            "['a-bc-d','two']",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        for s in [
            "['²','two']",
            "['\u{200c}','two']",
            "['\u{200d}','two']",
            "['a-b-c','two']",
            "['\u{1c89}','two']",
            "['\u{1e5d0}','two']",
            "[\"\\xFF\",'two']",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn static_payload_and_every_encoding_frozen_mutable_flag_survive_actual_edits() {
        fn values(s: &str) -> Vec<(Vec<u8>, bool, bool, bool, bool)> {
            let parsed = crate::parser::parse(s.as_bytes());
            #[derive(Default)]
            struct Values(Vec<(Vec<u8>, bool, bool, bool, bool)>);
            impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Values {
                fn enter(&mut self, n: Node<'pr>) {
                    if let Some(s) = n.as_string_node() {
                        self.0.push((
                            s.unescaped().to_vec(),
                            s.is_forced_utf8_encoding(),
                            s.is_forced_binary_encoding(),
                            s.is_frozen(),
                            s.is_mutable(),
                        ));
                    }
                }
            }
            let mut values = Values::default();
            crate::cops::walk::walk(&mut values, &parsed.node());
            values.0
        }
        for magic in [
            "",
            "# frozen_string_literal: false\n",
            "# frozen_string_literal: true\n",
            "# encoding: binary\n",
        ] {
            for s in [
                "['one','two']",
                "[\"\\u0061\",'two']",
                "[\"\\u4e2d\",'two']",
                "[?a, ?b]",
                "[\"one\\n\",\"hi\\tthere\"]",
                "%w[a\\ b \\]c]",
                "%W[a\\ b \\]c]",
                "%w[a\\ b\\\nc]",
                "%W[a\\ b\\\nc]",
                "%W[a\\ b\\v\\f\\a\\b\\e\\x00\\x1f]",
                "[\"one\n\", 'two']",
            ] {
                let s = format!("{magic}{s}");
                let found = crate::cops::testing::correction_offenses(&WordArray, &s);
                let after = crate::corrections::apply(s.as_bytes(), &found)
                    .unwrap()
                    .bytes;
                if magic.starts_with("# encoding") && values(&s).iter().any(|v| !v.0.is_ascii()) {
                    assert_eq!(s.as_bytes(), after);
                    continue;
                }
                assert_ne!(s.as_bytes(), after, "{s}");
                assert_eq!(
                    values(&s),
                    values(std::str::from_utf8(&after).unwrap()),
                    "{s}"
                );
            }
        }
    }
    #[test]
    fn lazy_builds_and_linear_matrix_classification_for_six_thousand_rows() {
        for n in [1500, 3000, 6000] {
            let s = format!("[{}['complex content','two']]", "['one','two'],".repeat(n));
            CLASSIFIED_BYTES.set(0);
            WORD_BUILDS.set(0);
            assert!(offenses(&s).is_empty());
            assert!(
                CLASSIFIED_BYTES.get() <= 8 * s.len(),
                "{} {}",
                s.len(),
                CLASSIFIED_BYTES.get()
            );
            assert_eq!(WORD_BUILDS.get(), 0);
            let s = "['one','two']\n".repeat(n);
            CLASSIFIED_BYTES.set(0);
            WORD_BUILDS.set(0);
            assert_eq!(offenses(&s).len(), n);
            assert_eq!(WORD_BUILDS.get(), 0);
            let found = crate::cops::testing::correction_offenses(&WordArray, &s);
            assert_eq!(found.len(), n);
            assert_eq!(WORD_BUILDS.get(), n);
            assert!(CLASSIFIED_BYTES.get() <= 8 * s.len());
            let mac = format!("# encoding: UTF8-MAC\n{}", "['e\u{301}','two']\n".repeat(n));
            CLASSIFIED_BYTES.set(0);
            WORD_BUILDS.set(0);
            assert_eq!(offenses(&mac).len(), n);
            assert_eq!(WORD_BUILDS.get(), 0);
            assert!(CLASSIFIED_BYTES.get() <= 8 * mac.len());
        }
    }
    #[test]
    fn pinned_brackets_diagnostic() {
        let o = offenses("# header\n  ['one', 'two', 'three']\n");
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 3, 2));
        assert_eq!(o[0].message, "Use `%w` or `%W` for an array of words.");
    }
    #[test]
    fn block_context_exemption_applies_only_to_direct_arguments() {
        for call in [
            ".each { |x| p x }",
            "&.each { |x| p x }",
            ".each do |x| p x end",
        ] {
            let source = format!("['one','two']{call}");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(found[0].message, "Use `%w` or `%W` for an array of words.");
            assert_correction(&WordArray, &source, &format!("%w[one two]{call}"));
        }
        for source in [
            "foo ['one','two'] { |x| p x }",
            "foo ['one','two'] do |x| p x end",
            "foo&.bar ['one','two'] { |x| p x }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn fixed_threshold_words_and_lookalikes() {
        for s in [
            "[]",
            "['one']",
            "['', 'two']",
            "['one space','two']",
            "['one', 2]",
            "['one', *x]",
            "['one', \"#{effect}\"]",
            "['-', '----']",
            "%w[one two]",
            "%w[% %i %I %q %Q %r %s %w %W %x]",
            "['foo', # comment\n 'bar']",
            "foo ['bar','baz'] { qux }",
            "[<<~A, 'two']\none\nA\n",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for s in [
            "['foo','bar','foo-bar']",
            "['ВУЗ','中文网']",
            "[\"one\\n\",\"hi\\tthere\"]",
            "['foo','bar'] # outside",
            "foo(['bar','baz']) { qux }",
            "[%q(one), ?a]",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
    }
    #[test]
    fn pinned_builder_static_fragment_boundaries_do_not_hide_matrix_peers() {
        for s in ["['a\nb', 'two']", "[%q||, 'two']", "[%Q||, 'two']"] {
            assert!(offenses(s).is_empty(), "{s}");
            assert_eq!(offenses(&format!("[{s}, ['a','b']]")).len(), 1, "{s}");
        }
        assert_eq!(offenses("[\"a\\\nb\", 'two']").len(), 1);
    }
    #[test]
    fn newline_delimited_word_lists_are_parser_errors_not_correctable_arrays() {
        // Unlike scalar percent strings, Ruby 3.4 word-list lexing consumes
        // these newlines as separators and cannot close a valid literal.
        for s in ["%w\na\\ b\n", "%w\r\na\\ b\r\n"] {
            assert!(crate::parser::parse(s.as_bytes()).errors().count() > 0);
            assert!(crate::cops::testing::correction_offenses(&WordArray, s).is_empty());
        }
    }
    #[test]
    fn matrix_policy() {
        assert_eq!(offenses("[['one','One'], ['two','Two']]").len(), 2);
        assert!(offenses("[['one','One'], ['two','Two'], ['forty two','Forty Two']]").is_empty());
        assert_eq!(offenses("[['one','One'], 2, 3]").len(), 1);
    }
    #[test]
    fn array_separator_continuations_never_become_percent_word_data() {
        for nl in ["\n", "\r\n"] {
            let before = format!("[ 'one', \\{nl} 'two', \\{nl} 'three' ]");
            let after = format!("%w[ one {nl} two {nl} three ]");
            assert_correction(&WordArray, &before, &after);
        }
    }
}
