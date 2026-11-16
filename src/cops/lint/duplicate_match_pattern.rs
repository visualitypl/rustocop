//! `Lint/DuplicateMatchPattern` — repeated raw-source patterns in a case match.
use std::{borrow::Cow, collections::HashSet};

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct DuplicateMatchPattern;

impl Cop for DuplicateMatchPattern {
    fn name(&self) -> &'static str {
        "Lint/DuplicateMatchPattern"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_case_match_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(case) = node.as_case_match_node() else {
            return;
        };
        let mut previous = HashSet::new();
        for branch in case.conditions().iter() {
            let branch = branch.as_in_node().unwrap();
            let mut pattern = branch.pattern();
            // Prism wraps guarded patterns in a modifier If/Unless. RuboCop
            // keeps the pattern and the complete `if/unless ...` source apart.
            let guard = if let Some(n) = pattern.as_if_node() {
                let body = n.statements().unwrap().body().first().unwrap();
                let start = n.if_keyword_loc().unwrap().start_offset();
                let end = n.location().end_offset();
                pattern = body;
                &file.bytes[start..end]
            } else if let Some(n) = pattern.as_unless_node() {
                let body = n.statements().unwrap().body().first().unwrap();
                let start = n.keyword_loc().start_offset();
                let end = n.location().end_offset();
                pattern = body;
                &file.bytes[start..end]
            } else {
                &[]
            };
            let mut key = identity(&pattern, file);
            key.extend_from_slice(&buffer_source(guard, file));
            if previous.insert(key) {
                continue;
            }
            let location = pattern.location();
            let (line, col) = buffer_line_col(location.start_offset(), file);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "Duplicate `in` pattern detected.".into(),
                line,
                col,
                end_line: file.last_line_of(location),
                correction: None,
            });
        }
    }
}

fn identity(pattern: &Node<'_>, file: &SourceFile) -> Vec<u8> {
    // These are RuboCop AST *direct* children, not a recursive fingerprint.
    // Constant-constrained hashes have an outer const_pattern and stay raw.
    let mut children = if let Some(hash) = pattern.as_hash_pattern_node() {
        if hash.constant().is_some() {
            return buffer_source(pattern.location().as_slice(), file);
        }
        let mut children: Vec<_> = hash.elements().iter().collect();
        children.extend(hash.rest());
        children
    } else if let Some(alt) = pattern.as_alternation_pattern_node() {
        vec![alt.left(), alt.right()]
    } else {
        return buffer_source(pattern.location().as_slice(), file);
    };
    let mut children: Vec<_> = children
        .drain(..)
        .map(|n| buffer_source(n.location().as_slice(), file))
        .collect();
    children.sort_unstable();
    // Ruby Array#to_s is an inspected array of strings. Keep its spelling,
    // including cross-kind collisions such as `0 | 1` and `["0", "1"]`.
    let mut key = vec![b'['];
    for (i, child) in children.iter().enumerate() {
        if i != 0 {
            key.extend_from_slice(b", ");
        }
        inspect_source(child, file, &mut key);
    }
    key.push(b']');
    key
}

// Parser::Source::Buffer projects declared encodings to UTF8, then normalizes
// physical CRLF. Do this before sorting/inspection and for raw patterns/guards
// alike. Keep original Prism offsets/bytes, projecting diagnostic columns too.
fn buffer_source(source: &[u8], file: &SourceFile) -> Vec<u8> {
    let projected = file.match_pattern_source(source);
    normalized_newlines(&projected).into_owned()
}

pub(super) fn buffer_line_col(offset: usize, file: &SourceFile) -> (u32, u32) {
    let line = file.line_of(offset);
    let start = if line == 1 && file.bytes.starts_with(b"\xef\xbb\xbf") {
        3.min(offset)
    } else {
        file.line_start(line)
    };
    // Composition, surrogate pairs and multi-scalar expansions can all change
    // the character count before the original AST offset. Use this cop's buffer
    // profile, not the main parser's declared-encoding character widths.
    let prefix = buffer_source(&file.bytes[start..offset], file);
    let columns = if file.match_pattern_binary() {
        prefix.len()
    } else {
        prefix
            .utf8_chunks()
            .map(|chunk| chunk.valid().chars().count() + chunk.invalid().len())
            .sum()
    };
    (line, columns as u32 + 1)
}

fn normalized_newlines(source: &[u8]) -> Cow<'_, [u8]> {
    if !source.windows(2).any(|pair| pair == b"\r\n") {
        return Cow::Borrowed(source);
    }
    Cow::Owned(
        source
            .iter()
            .enumerate()
            .filter_map(|(i, &b)| (b != b'\r' || source.get(i + 1) != Some(&b'\n')).then_some(b))
            .collect(),
    )
}

pub(crate) fn inspect_source(source: &[u8], file: &SourceFile, out: &mut Vec<u8>) {
    out.push(b'"');
    let mut i = 0;
    while i < source.len() {
        let b = source[i];
        // These are projected bytes, not bytes in the file's original encoding.
        // Binary buffers deliberately retain their byte-oriented inspection.
        let width = if file.match_pattern_binary() {
            1
        } else {
            match b {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => 1,
            }
            .min(source.len() - i)
        };
        let character = &source[i..i + width];
        i += width;
        let escape: &[u8] = match b {
            b'"' => b"\\\"",
            b'\\' => b"\\\\",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            b'\t' => b"\\t",
            7 => b"\\a",
            8 => b"\\b",
            11 => b"\\v",
            12 => b"\\f",
            27 => b"\\e",
            b'#' if source
                .get(i)
                .is_some_and(|c| matches!(c, b'{' | b'@' | b'$')) =>
            {
                b"\\#"
            }
            _ => {
                if file.match_pattern_binary() && !(0x20..0x7f).contains(&b) {
                    out.extend_from_slice(format!("\\x{b:02X}").as_bytes());
                } else if let Some(code) = file.inspection_nonprint(character) {
                    let escaped = if code <= 0xffff {
                        format!("\\u{code:04X}")
                    } else {
                        format!("\\u{{{code:X}}}")
                    };
                    out.extend_from_slice(escaped.as_bytes());
                } else {
                    out.extend_from_slice(character);
                }
                continue;
            }
        };
        out.extend_from_slice(escape);
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's lint/duplicate_match_pattern_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/DuplicateMatchPattern")
            .collect()
    }

    fn pair(a: &str, b: &str) -> String {
        format!("case x\nin {a}\n  first_method\nin {b}\n  second_method\nend\n")
    }

    #[test]
    fn flags_subsequent_raw_patterns_and_shallow_unordered_children() {
        for (a, b) in [
            ("foo", "foo"),
            ("0 | 1", "1 | 0"),
            ("[foo, bar]", "[foo, bar]"),
            ("foo: a, bar: b", "foo: a, bar: b"),
            ("foo: a, bar: b", "bar: b, foo: a"),
            ("foo if condition", "foo if condition"),
            ("foo unless condition", "foo unless condition"),
        ] {
            let source = pair(a, b);
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (4, 4, 4));
            assert_eq!(found[0].message, "Duplicate `in` pattern detected.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn ignores_nonduplicates_and_source_distinctions() {
        for (a, b) in [
            ("foo", "bar"),
            ("0 | 1 | 2", "3 | 4"),
            ("[foo, bar]", "[bar, foo]"),
            ("[foo, bar]", "[foo, bar, baz]"),
            ("foo: a, bar: b", "foo: a, bar: b, baz: c"),
            ("foo if condition1", "foo if condition2"),
            ("foo unless condition1", "foo unless condition2"),
            ("foo if condition", "foo unless condition"),
            ("foo", "foo if condition"),
            ("'first'", "\"first\""),
            ("1", "0x1"),
            ("[foo, bar]", "[foo,  bar]"),
            ("0 | 1 | 2", "2 | 1 | 0"),
            ("{a: {b: 1, c: 2}}", "{a: {c: 2, b: 1}}"),
            ("A[a: 1, b: 2]", "A[b: 2, a: 1]"),
            ("(0 | 1)", "(1 | 0)"),
        ] {
            let source = pair(a, b);
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for source in [
            "case x\nin false\n  first_method\nend\n",
            "case x\nin { key: value } if condition\nend\n",
            "case x\nin foo; method_name\nin bar; second_method\nelse; third_method\nend\n",
            "x in [a]\nx in [a]\nx => [a]\nx => [a]\n",
            "case x; when 1; first; when 1; second; end\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn selectors_cannot_bypass_minimum_ruby_gate() {
        use crate::cops::{Registry, policy::RubyVersion, select_active_for_version};
        let registry = Registry::new();
        let config = Default::default();
        for (version, count) in [
            (RubyVersion(2, 6), 0),
            (RubyVersion(2, 7), 1),
            (RubyVersion(3, 4), 1),
        ] {
            let active = select_active_for_version(
                &registry,
                &config,
                &["Lint/DuplicateMatchPattern".into()],
                &[],
                version,
            );
            assert_eq!(active.len(), count);
        }
    }

    #[test]
    fn pinned_specs_sources_and_identity_controls() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/duplicate_match_pattern.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 148);
        for row in rows {
            let source = row["source"].as_str().unwrap();
            let actual: Vec<_> = offenses(source)
                .iter()
                .map(|o| {
                    assert!(o.correction.is_none());
                    serde_json::json!([o.line, o.col, o.end_line, o.message])
                })
                .collect();
            assert_eq!(actual, *row["findings"].as_array().unwrap(), "{row}");
        }
    }

    #[test]
    fn unicode15_inspected_arrays_do_not_use_newer_host_assignments() {
        for code in [0x1fae9, 0x1c89, 0x1cc00] {
            let c = char::from_u32(code).unwrap();
            let escaped = if code <= 0xffff {
                format!("\\u{code:04X}")
            } else {
                format!("\\u{{{code:X}}}")
            };
            for (second, expected) in [(escaped, 1), (c.to_string(), 0)] {
                let source = pair(&format!("'{c}' | 1"), &format!("[\"'{second}'\", \"1\"]"));
                let found = offenses(&source);
                assert_eq!(found.len(), expected, "{source}");
                if expected == 1 {
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (4, 4, 4));
                    assert_eq!(found[0].message, "Duplicate `in` pattern detected.");
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }

    #[test]
    fn reports_every_repeat_in_order_and_keeps_case_state_local() {
        let found = offenses(
            "case x\nin foo; first\nin bar; second\nin foo; third\nin bar; fourth\nin foo; fifth\nend\ncase y; in foo; other; end\n",
        );
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(4, 4), (5, 4), (6, 4)]
        );
    }
}
