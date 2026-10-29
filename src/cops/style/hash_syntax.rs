//! `Style/HashSyntax` — flags hash rockets (`:a => 1`) where Ruby 1.9 syntax
//! (`a: 1`) would do. rustocop's fixed style is `ruby19`.
//!
//! Matches RuboCop's semantics for that style with its defaults
//! (`EnforcedShorthandSyntax: either`, so `{ a: }` and `{ a: a }` are both
//! fine; `UseHashRocketsWithSymbolValues` and
//! `PreferHashRocketsForNonAlnumEndingSymbols` off). A hash or keyword
//! argument list is checked only when every key is a symbol that 1.9 syntax
//! can spell: an ASCII word (`a`, `_b`, `C`, `d?`, `e!`, also allowing
//! Unicode case-folded initials `K` and `ſ`) or a quoted
//! symbol (`:"x y"`, `:'&&'`). Then each pair written with `=>` is reported
//! at its key. A single other key (`"b" => 1`, `:+ => 1`, `1 => 2`) makes
//! the whole hash acceptable; double splats (`**opts`) are not keys.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

/// The `Style/HashSyntax` cop.
pub struct HashSyntax;

impl Cop for HashSyntax {
    fn name(&self) -> &'static str {
        "Style/HashSyntax"
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
        let elements = match node {
            Node::HashNode { .. } => node.as_hash_node().unwrap().elements(),
            Node::KeywordHashNode { .. } => node.as_keyword_hash_node().unwrap().elements(),
            _ => return,
        };
        let pairs: Vec<_> = elements
            .iter()
            .filter_map(|element| element.as_assoc_node())
            .collect();
        if pairs.is_empty() || !pairs.iter().all(|pair| is_word_symbol_key(&pair.key())) {
            return;
        }
        let missing_space = cx.ancestors().any(|ancestor| {
            ancestor.as_call_node().is_some_and(|call| {
                call.message_loc()
                    .is_some_and(|loc| loc.end_offset() == node.location().start_offset())
            })
        });
        let corrections: Vec<_> = pairs
            .iter()
            .filter(|pair| pair.operator_loc().is_some())
            .map(|pair| {
                pair_correction(
                    pair,
                    file,
                    missing_space
                        && pair.key().location().start_offset() == node.location().start_offset(),
                )
            })
            .collect();
        let needs_braces = node.as_keyword_hash_node().is_some()
            && cx
                .ancestors()
                .find(|ancestor| ancestor.as_arguments_node().is_none())
                .is_some_and(|ancestor| {
                    matches!(
                        ancestor,
                        Node::ReturnNode { .. } | Node::NextNode { .. } | Node::BreakNode { .. }
                    )
                });
        let braced_correction = if needs_braces {
            corrections
                .iter()
                .cloned()
                .collect::<Option<Vec<_>>>()
                .and_then(|mut edits| {
                    let loc = node.location();
                    edits.push(Correction {
                        range: loc.start_offset()..loc.start_offset(),
                        replacement: "{".into(),
                    });
                    edits.push(Correction {
                        range: loc.end_offset()..loc.end_offset(),
                        replacement: "}".into(),
                    });
                    super::correction::combine(file, edits)
                })
        } else {
            None
        };
        for (pair, correction) in pairs
            .iter()
            .filter(|pair| pair.operator_loc().is_some())
            .zip(corrections)
        {
            let (line, col) = file.line_col(pair.key().location().start_offset());
            out.push(Offense {
                correction: if needs_braces {
                    braced_correction.clone()
                } else {
                    correction
                },
                cop: self.name(),
                severity: self.severity(),
                message: "Use the new Ruby 1.9 hash syntax.".to_string(),
                line,
                end_line: file.last_line_of(pair.operator_loc().unwrap()),
                col,
            });
        }
    }
}

fn pair_correction(
    pair: &ruby_prism::AssocNode<'_>,
    file: &SourceFile,
    space: bool,
) -> Option<Correction> {
    let key = pair.key();
    let loc = key.location();
    let operator = pair.operator_loc()?;
    let mut end = operator.end_offset();
    while file
        .bytes
        .get(end)
        .is_some_and(|b| matches!(b, b' ' | b'\t'))
    {
        end += 1;
    }
    let gap = &file.bytes[loc.end_offset()..operator.start_offset()];
    let mut replacement = if space { " ".to_owned() } else { String::new() };
    replacement.push_str(std::str::from_utf8(loc.as_slice().strip_prefix(b":")?).ok()?);
    replacement.push(':');
    // Preserve comments and their newlines between the key and rocket.
    if !gap.iter().all(u8::is_ascii_whitespace) {
        replacement.push_str(std::str::from_utf8(gap).ok()?);
    }
    replacement.push(' ');
    Some(Correction {
        range: loc.start_offset()..end,
        replacement,
    })
}

/// RuboCop's `word_symbol_pair?`: a symbol key that 1.9 syntax can spell.
fn is_word_symbol_key(key: &Node<'_>) -> bool {
    let closing = match key {
        Node::SymbolNode { .. } => key.as_symbol_node().unwrap().closing_loc(),
        Node::InterpolatedSymbolNode { .. } => {
            key.as_interpolated_symbol_node().unwrap().closing_loc()
        }
        _ => return false,
    };
    // The key as the parser gem sees it: without a label's trailing colon
    // (`a:`, `"a b":`) or a symbol's leading one (`:a`).
    let source = key.location().as_slice();
    let source = match closing {
        Some(closing) if closing.as_slice().ends_with(b":") => &source[..source.len() - 1],
        _ => source,
    };
    is_acceptable_19_syntax_symbol(source.strip_prefix(b":").unwrap_or(source))
}

/// RuboCop's `acceptable_19_syntax_symbol?`: `\A[_a-z]\w*[?!]?\z` (ignoring
/// case, including Unicode folds of the initial letter), or a quoted symbol name.
fn is_acceptable_19_syntax_symbol(name: &[u8]) -> bool {
    let quoted = name.len() >= 2
        && ((name.starts_with(b"\"") && name.ends_with(b"\""))
            || (name.starts_with(b"'") && name.ends_with(b"'")));
    if quoted {
        return true;
    }
    let word = name
        .strip_suffix(b"?")
        .or_else(|| name.strip_suffix(b"!"))
        .unwrap_or(name);
    let Ok(word) = std::str::from_utf8(word) else {
        return false;
    };
    let mut chars = word.chars();
    // Ruby's /i folds [_a-z] to include Kelvin sign and long s, but \w
    // remains ASCII even with /i. Do not accept arbitrary Unicode letters.
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || matches!(first, '_' | 'K' | 'ſ'))
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/style/hash_syntax_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the `ruby19`
    //! style with the default `EnforcedShorthandSyntax: either`.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/HashSyntax")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn corrects_ruby19_hashes() {
        // RuboCop expect_correction examples (MIT), ruby19/either.
        for (before, after) in [
            ("x = { :a => 0, :b   =>  2}\n", "x = { a: 0, b: 2}\n"),
            ("func(3, :a => 0)\n", "func(3, a: 0)\n"),
            ("{ :'&&' => foo }\n", "{ '&&': foo }\n"),
            ("{ :\"a#{b}\" => 1 }\n", "{ \"a#{b}\": 1 }\n"),
            ("foo:bar => 1\n", "foo bar: 1\n"),
            ("return :a => 1, :b => 2\n", "return {a: 1, b: 2}\n"),
            ("return {:key => value}\n", "return {key: value}\n"),
            ("{ :a=>1, :b=>2 }\n", "{ a: 1, b: 2 }\n"),
            ("{ :a => # value\n 1 }\n", "{ a: # value\n 1 }\n"),
        ] {
            crate::cops::testing::assert_correction(&HashSyntax, before, after);
        }
    }

    #[test]
    fn flags_unicode_case_folded_initials_in_mixed_hashes() {
        for key in ["K", "ſ", "K1", "ſ_?", "K!"] {
            let source = format!("h = {{ :a => 0, :{key} => 1 }}\n");
            let found = offenses(&source);
            assert_eq!(positions(&source), vec![(1, 7), (1, 16)], "{source}");
            for offense in found {
                assert_eq!(offense.message, "Use the new Ruby 1.9 hash syntax.");
            }
            crate::cops::testing::assert_correction(
                &HashSyntax,
                &source,
                &format!("h = {{ a: 0, {key}: 1 }}\n"),
            );
            assert!(offenses(&format!("h = {{ a: 0, {key}: 1 }}\n")).is_empty());
            // A label with a folded initial must not suppress another rocket.
            assert_eq!(
                positions(&format!("h = {{ :a => 0, {key}: 1 }}\n")),
                vec![(1, 7)]
            );
        }
    }

    #[test]
    fn rejects_unicode_word_lookalikes_in_mixed_hashes() {
        // Ruby's /i folds the initial [_a-z], but not the remaining \w.
        for key in ["aK", "aſ", "Kſ", "é", "İ", "ı", "ß", "ﬀ"] {
            let source = format!("h = {{ :a => 0, :{key} => 1 }}\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn corrects_braceless_control_flow_hashes_atomically() {
        for (before, after) in [
            (
                "p [1].map { next :a => 1 }\n",
                "p [1].map { next {a: 1} }\n",
            ),
            (
                "loop { break :a => 1, :b => 2 }\n",
                "loop { break {a: 1, b: 2} }\n",
            ),
            ("loop { break {:a => 1} }\n", "loop { break {a: 1} }\n"),
            (
                "[1].map { next({:a => 1}) }\n",
                "[1].map { next({a: 1}) }\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&HashSyntax, before, after);
        }
    }

    #[test]
    fn flags_hash_rockets_with_message() {
        let found = offenses("x = { :a => 0, :b   =>  2}\n");
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].col), (1, 7));
        assert_eq!((found[1].line, found[1].col), (1, 16));
        assert_eq!(found[0].message, "Use the new Ruby 1.9 hash syntax.");
    }

    #[test]
    fn flags_rubocop_spec_offenses() {
        for (source, expected) in [
            ("x = { :a => 0, b: 1 }\n", vec![(1, 7)]),
            ("func(3, :a => 0)\n", vec![(1, 9)]),
            ("x = { :\"string\" => 0 }\n", vec![(1, 7)]),
            ("{ :'&&' => foo }\n", vec![(1, 3)]),
            ("x = { :a? => 0 }\n", vec![(1, 7)]),
            ("x = { :a! => 0 }\n", vec![(1, 7)]),
            ("return :key => value\n", vec![(1, 8)]),
            ("return {:key => value}\n", vec![(1, 9)]),
            ("return :a => 1, :b => 2\n", vec![(1, 8), (1, 17)]),
            ("method(:puts).(:key => value)\n", vec![(1, 16)]),
            ("{ :a=>1, :b=>2 }\n", vec![(1, 3), (1, 10)]),
        ] {
            assert_eq!(positions(source), expected, "{source:?}");
        }
    }

    #[test]
    fn accepts_rubocop_spec_examples() {
        for source in [
            "x = { :a => 0, \"b\" => 1 }\n",
            "{}\n",
            "x = { a: 0, b: 1 }\n",
            // Shorthand syntax is accepted either way (`either`).
            "if foo bar: bar\n  baz\nend\n",
            "baz if foo bar: bar\n",
            "{ a:, b: }\n",
            "{ a:, b: b }\n",
        ] {
            assert!(offenses(source).is_empty(), "should accept {source:?}");
        }
    }

    #[test]
    fn flags_other_symbol_keys() {
        for (source, expected) in [
            ("foo(:a => 1)\n", vec![(1, 5)]),
            ("foo :a => 1, :b => 2\n", vec![(1, 5), (1, 14)]),
            ("{ :_a => 1, :B1 => 2 }\n", vec![(1, 3), (1, 13)]),
            ("{ :\"a#{b}\" => 1 }\n", vec![(1, 3)]),
            ("{ **opts, :a => 1 }\n", vec![(1, 11)]),
            ("foo[:a => 1]\n", vec![(1, 5)]),
            ("{ \"x y\": 1, :z => 2 }\n", vec![(1, 13)]),
            ("{\n  :a => 1,\n  b: 2\n}\n", vec![(2, 3)]),
        ] {
            assert_eq!(positions(source), expected, "{source:?}");
        }
    }

    #[test]
    fn accepts_lookalikes() {
        for source in [
            "{ \"a\" => 1 }\n",
            "{ 1 => 2, :a => 3 }\n",
            "{ :+ => 1 }\n",
            "{ :a= => 1 }\n",
            "{ :@a => 1, :b => 2 }\n",
            "{ :$a => 1 }\n",
            "{ :\"a\" => 1, b => 2 }\n",
            "{ %s(a) => 1 }\n",
            "{ :é => 1, :b => 2 }\n",
            "{ é: 1, :b => 2 }\n",
            "{ a => 1 }\n",
            "{ A => 1 }\n",
            "{ **opts }\n",
            "case x\nin { a: 1 }\nend\n",
            "foo(a: 1, b: 2)\n",
        ] {
            assert!(offenses(source).is_empty(), "should accept {source:?}");
        }
    }

    #[test]
    fn honors_directives() {
        assert!(offenses("{ :a => 1 } # rubocop:disable Style/HashSyntax\n").is_empty());
    }
}
