//! `Naming/InclusiveLanguage` — opt-in, fixed pinned three-term policy.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct InclusiveLanguage;
impl Cop for InclusiveLanguage {
    fn name(&self) -> &'static str {
        "Naming/InclusiveLanguage"
    }
    fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
        let path = file.path.to_string_lossy();
        let matches = scan(
            path.as_bytes(),
            |b| (b[0].leading_ones() as usize).max(1),
            |b| match b {
                [0xc5, 0xbf] => [b's', 0],
                [0xe2, 0x84, 0xaa] => [b'k', 0],
                [0xef, 0xac, 0x85 | 0x86] => *b"st",
                _ => [0, 0],
            },
        );
        if !matches.is_empty() {
            let message = if matches.len() == 1 {
                let (start, end, term) = matches[0];
                format!(
                    "Consider replacing '{}' in file path{}.",
                    &path[start..end],
                    suggestions(term)
                )
            } else {
                let words: Vec<_> = matches.iter().map(|&(s, e, _)| &path[s..e]).collect();
                format!(
                    "Consider replacing '{}' in file path with other terms.",
                    words.join("', '")
                )
            };
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line: 1,
                end_line: 1,
                col: 1,
                message,
                correction: None,
            });
        }
        for range in file.comments() {
            self.check_range(file, range.clone(), out);
        }
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if let Some((start, end, _)) = super::ascii_identifiers::token(node, cx, file) {
            self.check_range(file, start..end, out);
            return;
        }
        // Quoted/dynamic symbols and percent symbol arrays use STRING_CONTENT,
        // not tSYMBOL. Labels and implicit shorthand values are not identifiers.
        if let Some(symbol) = node.as_symbol_node() {
            if symbol.opening_loc().is_some_and(|l| l.as_slice() == b":")
                && let Some(loc) = symbol.value_loc()
            {
                self.check_range(file, loc.start_offset()..loc.end_offset(), out);
            }
            return;
        }
        macro_rules! read {
            ($cast:ident) => {
                if let Some(n) = node.$cast() {
                    self.check_range(
                        file,
                        n.location().start_offset()..n.location().end_offset(),
                        out,
                    );
                    return;
                }
            };
        }
        read!(as_instance_variable_read_node);
        read!(as_class_variable_read_node);
        read!(as_global_variable_read_node);
        read!(as_global_variable_target_node);
        macro_rules! write {
            ($cast:ident) => {
                if let Some(n) = node.$cast() {
                    let l = n.name_loc();
                    self.check_range(file, l.start_offset()..l.end_offset(), out);
                    return;
                }
            };
        }
        write!(as_global_variable_write_node);
        write!(as_global_variable_or_write_node);
        write!(as_global_variable_and_write_node);
        write!(as_global_variable_operator_write_node);
        if let Some((name, start)) = super::variable_name::variable_name(node)
            && matches!(name.first(), Some(b'@' | b'$'))
        {
            self.check_range(file, start..start + name.len(), out);
        }
    }
}

impl InclusiveLanguage {
    fn check_range(
        &self,
        file: &SourceFile,
        range: std::ops::Range<usize>,
        out: &mut Vec<Offense>,
    ) {
        let bytes = &file.bytes[range.clone()];
        let mut seen = std::collections::HashSet::new();
        for (start, end, term) in scan(
            bytes,
            |b| file.encoded_character_width(b),
            |b| file.inclusive_case_fold(b),
        ) {
            let word = &bytes[start..end];
            if !seen.insert(word) {
                continue;
            }
            // The pinned range helper uses token.source.index(word), even for
            // repeated identical spellings in the same comment token.
            // String#index operates on decoded characters in the pin. Do not
            // let an earlier ASCII trail byte of a legacy character supply it.
            let mut first = 0;
            while !bytes[first..].starts_with(word) {
                first += file.encoded_character_width(&bytes[first..]).max(1);
            }
            let (line, col) = file.line_col(range.start + first);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                end_line: file.line_of(range.start + first + word.len() - 1),
                col,
                message: format!(
                    "Consider replacing '{}'{}.",
                    String::from_utf8_lossy(word),
                    suggestions(term)
                ),
                correction: None,
            });
        }
    }
}

fn suggestions(term: usize) -> &'static str {
    match term {
        0 => " with 'allowlist' or 'permit'",
        1 => " with 'denylist' or 'block'",
        _ => " with 'replica', 'secondary', or 'follower'",
    }
}

/// RuboCop extracts Regexp#source (discarding the serialized (?-mix:) flags)
/// then compiles the combined pattern with IGNORECASE. All three terms fold.
/// Ruby's [\\W_] permits non-ASCII neighbours and underscores. Keep original
/// character widths/offsets rather than matching an ASCII legacy trail byte.
fn scan(
    bytes: &[u8],
    width: impl Fn(&[u8]) -> usize,
    nonascii_fold: impl Fn(&[u8]) -> [u8; 2],
) -> Vec<(usize, usize, usize)> {
    let mut units = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let length = width(&bytes[at..]).max(1).min(bytes.len() - at);
        let raw = &bytes[at..at + length];
        let folded = if length == 1 && raw[0].is_ascii() {
            [raw[0].to_ascii_lowercase(), 0]
        } else {
            nonascii_fold(raw)
        };
        units.push((
            at,
            at + length,
            folded[0],
            length == 1 && raw[0].is_ascii_alphanumeric(),
        ));
        if folded[1] != 0 {
            units.push((at, at + length, folded[1], false));
        }
        at += length;
    }
    let mut result = Vec::new();
    let mut i = 0;
    while i < units.len() {
        let matches = |start: usize, word: &[u8]| {
            units
                .get(start..start + word.len())
                .is_some_and(|s| s.iter().map(|u| u.2).eq(word.iter().copied()))
        };
        let mut matched = None;
        for (term, prefix) in [b"white".as_slice(), b"black".as_slice()]
            .iter()
            .enumerate()
        {
            if matches(i, prefix) {
                let after = i + prefix.len();
                let gap = usize::from(units.get(after).is_some_and(|u| {
                    matches!(
                        u.2,
                        b'-' | b'_' | b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c
                    )
                }));
                if matches(after + gap, b"list") {
                    matched = Some((prefix.len() + gap + 4, term));
                    break;
                }
            }
        }
        if matched.is_none()
            && matches(i, b"slave")
            && (i == 0 || !units[i - 1].3)
            && !units.get(i + 5).is_some_and(|u| u.3)
        {
            matched = Some((5, 2));
        }
        if let Some((length, term)) = matched {
            result.push((units[i].0, units[i + length - 1].1, term));
            i += length;
        } else {
            i += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    //! Ported/adapted from RuboCop naming/inclusive_language_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Fixed defaults only.
    use super::*;
    use crate::pipeline::{lint_str, lint_str_with};
    fn found(source: &str) -> Vec<Offense> {
        lint_str_with(&[&InclusiveLanguage], source)
    }
    #[test]
    fn every_pinned_source_and_controlled_token_example_at_fixed_defaults() {
        for fixture in [
            include_str!("../../../tests/fixtures/ns09_pinned.json"),
            include_str!("../../../tests/fixtures/ns09_source.json"),
            include_str!("../../../tests/fixtures/ns09_syntax.json"),
            include_str!("../../../tests/fixtures/ns09_paths.json"),
        ] {
            let rows: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
            for row in rows {
                let source = row["source"].as_str().unwrap();
                let path = std::path::Path::new(row["file"].as_str().unwrap_or("a.rb"));
                let report = crate::pipeline::lint_one(
                    path,
                    path,
                    Some(source.as_bytes()),
                    &[&InclusiveLanguage],
                    &crate::config::ResolvedConfig::default(),
                );
                assert!(report.errors.is_empty(), "{row}: {:?}", report.errors);
                let positions: Vec<_> = report
                    .result
                    .offenses
                    .iter()
                    .map(|o| vec![o.line, o.col])
                    .collect();
                let mut expected = row["positions"].clone();
                // Shared rustocop columns exclude the BOM signature; the pin
                // counts it as one character. Preserve the accepted convention.
                if source.starts_with('\u{feff}') {
                    for p in expected.as_array_mut().unwrap() {
                        if p[0] == 1 {
                            p[1] = serde_json::json!(p[1].as_u64().unwrap() - 1);
                        }
                    }
                }
                assert_eq!(serde_json::json!(positions), expected, "{row}");
                let messages: Vec<_> = report
                    .result
                    .offenses
                    .iter()
                    .map(|o| o.message.as_str())
                    .collect();
                assert_eq!(serde_json::json!(messages), row["messages"], "{row}");
                assert!(
                    report
                        .result
                        .offenses
                        .iter()
                        .all(|o| o.correction.is_none())
                );
            }
        }
    }
    #[test]
    fn pinned_legacy_projection_widths_and_unicode_folds_keep_raw_offsets() {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns09_encoding.json"))
                .unwrap();
        for row in rows {
            let hex = row["source_hex"].as_str().unwrap();
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            let path = std::path::Path::new("a.rb");
            let report = crate::pipeline::lint_one(
                path,
                path,
                Some(&bytes),
                &[&InclusiveLanguage],
                &Default::default(),
            );
            assert!(report.errors.is_empty(), "{row}: {:?}", report.errors);
            let positions: Vec<_> = report
                .result
                .offenses
                .iter()
                .map(|o| vec![o.line, o.col])
                .collect();
            let expected = row.get("rustocop_positions").unwrap_or(&row["positions"]);
            assert_eq!(&serde_json::json!(positions), expected, "{row}");
            assert!(
                report
                    .result
                    .offenses
                    .iter()
                    .all(|o| o.correction.is_none())
            );
        }
    }
    #[test]
    fn fixed_patterns_case_and_whole_word_boundaries() {
        let found = found(
            "whitelist = 1\n@black_list = 2\n# white-list black list slave Slave SLAVE database_slave\n",
        );
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 1), (2, 2), (3, 3), (3, 14), (3, 25), (3, 31), (3, 37)]
        );
        assert_eq!(
            found[0].message,
            "Consider replacing 'whitelist' with 'allowlist' or 'permit'."
        );
        assert_eq!(
            found[1].message,
            "Consider replacing 'black_list' with 'denylist' or 'block'."
        );
        assert_eq!(
            found[4].message,
            "Consider replacing 'slave' with 'replica', 'secondary', or 'follower'."
        );
        assert!(found.iter().all(|o| o.correction.is_none()));
    }
    #[test]
    fn opt_in_and_unselected_token_lookalikes() {
        assert!(
            lint_str("whitelist = 1\n")
                .iter()
                .all(|o| o.cop != InclusiveLanguage.name())
        );
        for source in [
            "TeslaVehicle; SLAVersion; DatabaseSlave; Slave1",
            "'whitelist slave'; :\"blacklist\"; %i[slave whitelist]",
            "foo(whitelist: 1); {slave: 1}",
            "def self.whitelist?; end; whitelist!",
            "__END__\n# whitelist slave\n",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn multi_character_case_folds_and_repeated_token_range_quirk() {
        for term in ["whiteliﬆ", "whiteliﬅ", "BlacKliſt"] {
            assert_eq!(found(&format!("# {term}\n")).len(), 1);
        }
        let found = found("# enslave slave slave Slave\n");
        assert_eq!(found.iter().map(|o| o.col).collect::<Vec<_>>(), [5, 23]);
        // The term alphabet/case/one optional gap is finite: no per-word AST
        // rescans or growing correction indexes, even for repeated comments.
        assert_eq!(
            self::found(&format!("# {}\n", "whitelist slave ".repeat(10_000))).len(),
            2
        );
    }
    #[test]
    fn unicode_case_folding_and_nonascii_word_neighbours() {
        for name in ["UTF-8", "UTF8-MAC", "CESU-8", "UTF8-DoCoMo"] {
            let source = format!("# coding: {name}\n# blacKliſt ſlave éslave slave奴\n");
            let found = found(&source);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                [(2, 3), (2, 13), (2, 20)]
            );
        }
    }
    #[test]
    fn selected_identifiers_variables_symbols_and_comments() {
        for (source, col) in [
            ("whitelist = 1", 1),
            ("class Nodewhitelist; end", 11),
            ("@whitelist", 2),
            ("@@blacklist", 3),
            ("$slave", 2),
            ("config[:slave] = {}", 9),
            ("def whitelist?; end", 5),
            ("alias whitelist blacklist", 7),
            ("=begin\nslave\n=end", 1),
        ] {
            let found = found(source);
            assert!(!found.is_empty(), "{source}");
            assert_eq!(found[0].col, col, "{source}");
        }
    }
}
