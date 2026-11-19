//! `Style/HashSyntax` — flags hash rockets (`:a => 1`) where Ruby 1.9 syntax
//! (`a: 1`) would do, or the other way round. `enforced_style` is RuboCop's
//! `EnforcedStyle`, with `ruby19` as the default.
//!
//! Matches RuboCop's semantics with its other defaults
//! (`EnforcedShorthandSyntax: either`, so `{ a: }` and `{ a: a }` are both
//! fine; `UseHashRocketsWithSymbolValues` and
//! `PreferHashRocketsForNonAlnumEndingSymbols` off). A hash or keyword
//! argument list has only symbol keys when every key is a symbol that 1.9
//! syntax can spell: an ASCII word (`a`, `_b`, `C`, `d?`, `e!`, also allowing
//! Unicode case-folded initials `K` and `ſ`) or a quoted
//! symbol (`:"x y"`, `:'&&'`). A single other key (`"b" => 1`, `:+ => 1`,
//! `1 => 2`) makes it a hash of mixed keys; double splats (`**opts`) are not
//! keys. A pair is reported at its key:
//!
//! - `ruby19`: each pair with `=>` in a hash of symbol keys.
//! - `hash_rockets`: each pair with a colon.
//! - `ruby19_no_mixed_keys`: as `ruby19` in a hash of symbol keys; each pair
//!   with a colon in a hash of mixed keys.
//! - `no_mixed_keys`: in a hash of symbol keys, each pair that is not written
//!   as the first one is; each pair with a colon in a hash of mixed keys.
//!
//! A shorthand pair named after a keyword (`if:`) has no hash rocket form and
//! is not reported.

use ruby_prism::Node;

use crate::{
    cops::{
        Cop, NodeContext,
        settings::{Kind, Setting, Values},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

/// The `Style/HashSyntax` cop.
#[derive(Default)]
pub struct HashSyntax {
    style: Style,
}

/// `enforced_style`.
#[derive(Clone, Copy, Default)]
enum Style {
    #[default]
    Ruby19,
    HashRockets,
    NoMixedKeys,
    Ruby19NoMixedKeys,
}

const MSG_19: &str = "Use the new Ruby 1.9 hash syntax.";
const MSG_HASH_ROCKETS: &str = "Use hash rockets syntax.";
const MSG_NO_MIXED_KEYS: &str = "Don't mix styles in the same hash.";

/// The keywords of RuboCop's `HashShorthandSyntax`: `{ if: }` cannot be
/// written as `{ :if => if }`.
const KEYWORDS: &[&str] = &[
    "__ENCODING__",
    "__FILE__",
    "__LINE__",
    "BEGIN",
    "END",
    "alias",
    "and",
    "begin",
    "break",
    "case",
    "class",
    "def",
    "do",
    "else",
    "elsif",
    "end",
    "ensure",
    "false",
    "for",
    "if",
    "in",
    "module",
    "next",
    "nil",
    "not",
    "or",
    "redo",
    "rescue",
    "retry",
    "return",
    "self",
    "super",
    "then",
    "true",
    "undef",
    "unless",
    "until",
    "when",
    "while",
    "yield",
];

impl Cop for HashSyntax {
    fn name(&self) -> &'static str {
        "Style/HashSyntax"
    }

    fn settings(&self) -> &'static [Setting] {
        const ENFORCED_STYLE: Setting = Setting {
            name: "enforced_style",
            kind: Kind::Choice(&[
                "ruby19",
                "hash_rockets",
                "no_mixed_keys",
                "ruby19_no_mixed_keys",
            ]),
        };
        &[ENFORCED_STYLE]
    }

    fn configure(&self, values: &Values) -> Option<Box<dyn Cop>> {
        let style = match values.choice("enforced_style") {
            Some("hash_rockets") => Style::HashRockets,
            Some("no_mixed_keys") => Style::NoMixedKeys,
            Some("ruby19_no_mixed_keys") => Style::Ruby19NoMixedKeys,
            Some(_) => Style::Ruby19,
            None => self.style,
        };
        Some(Box::new(HashSyntax { style }))
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
        let Some(first) = pairs.first() else { return };
        let rocket = |pair: &ruby_prism::AssocNode<'_>| pair.operator_loc().is_some();
        let symbols = pairs.iter().all(|pair| is_word_symbol_key(&pair.key()));
        // Whether the pairs with `=>` are the ones to report, or the others.
        let (rockets, message) = match (self.style, symbols) {
            (Style::Ruby19, false) => return,
            (Style::Ruby19 | Style::Ruby19NoMixedKeys, true) => (true, MSG_19),
            (Style::HashRockets, _) => (false, MSG_HASH_ROCKETS),
            (Style::NoMixedKeys, true) => (!rocket(first), MSG_NO_MIXED_KEYS),
            (Style::NoMixedKeys | Style::Ruby19NoMixedKeys, false) => (false, MSG_NO_MIXED_KEYS),
        };
        if !rockets {
            for pair in pairs.iter().filter(|pair| !rocket(pair)) {
                let key = pair.key();
                let omitted = pair.value().as_implicit_node().is_some();
                let keyword = key.as_symbol_node().is_some_and(|symbol| {
                    KEYWORDS
                        .iter()
                        .any(|keyword| keyword.as_bytes() == symbol.unescaped())
                });
                if omitted && keyword {
                    continue;
                }
                let (line, col) = file.line_col(key.location().start_offset());
                out.push(Offense {
                    correction: rocket_correction(pair, omitted, file),
                    cop: self.name(),
                    severity: self.severity(),
                    message: message.to_string(),
                    line,
                    end_line: file.last_line_of(key.location()),
                    col,
                });
            }
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
        // Upstream wraps the hash in braces in the first pair's correction
        // only, and each enabled pair converts itself. One correction per
        // offense: the first pair's carries the braces and every enabled
        // pair's edit, so a single pass gives upstream's result.
        let enabled = |pair: &ruby_prism::AssocNode<'_>| {
            file.cop_enabled_for_lines(
                self.name(),
                file.line_of(pair.key().location().start_offset()),
                file.last_line_of(pair.operator_loc().unwrap()),
            )
        };
        let braced = needs_braces && rocket(first) && enabled(first);
        let braced_correction = if braced {
            let mut edits: Vec<_> = pairs
                .iter()
                .filter(|pair| pair.operator_loc().is_some())
                .zip(&corrections)
                .filter(|(pair, _)| enabled(pair))
                .filter_map(|(_, correction)| correction.clone())
                .collect();
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
        } else {
            None
        };
        for (index, (pair, correction)) in pairs
            .iter()
            .filter(|pair| pair.operator_loc().is_some())
            .zip(corrections)
            .enumerate()
        {
            let (line, col) = file.line_col(pair.key().location().start_offset());
            out.push(Offense {
                correction: match (braced, index) {
                    (true, 0) => braced_correction.clone(),
                    (true, _) => None,
                    (false, _) => correction,
                },
                cop: self.name(),
                severity: self.severity(),
                message: message.to_string(),
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
    let end = value_start(file, operator.end_offset(), true);
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

/// The end of what a correction replaces after the operator that ends at
/// `end`: RuboCop's `range_with_surrounding_space`, the spaces and tabs and
/// then, with `newlines`, the line breaks. A value on the next line so
/// joins the line of its key.
fn value_start(file: &SourceFile, mut end: usize, newlines: bool) -> usize {
    let mut skip = |bytes: &[u8]| {
        while file.bytes.get(end).is_some_and(|b| bytes.contains(b)) {
            end += 1;
        }
    };
    skip(b" \t");
    if newlines {
        skip(b"\n");
    }
    end
}

/// `key: value` as `:key => value`, and `key:` as `:key => key`, which keeps
/// the line break after it.
fn rocket_correction(
    pair: &ruby_prism::AssocNode<'_>,
    omitted: bool,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = pair.key().location();
    let key = std::str::from_utf8(loc.as_slice().strip_suffix(b":")?).ok()?;
    Some(Correction {
        range: loc.start_offset()..value_start(file, loc.end_offset(), !omitted),
        replacement: format!(":{key} => {}", if omitted { key } else { "" }),
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
            // The value joins the line of its key, as in RuboCop.
            ("{ :a =>\n  1 }\n", "{ a:   1 }\n"),
        ] {
            crate::cops::testing::assert_correction(&HashSyntax::default(), before, after);
        }
    }

    fn styled(style: Style, source: &str) -> Vec<(u32, u32, String)> {
        let found = crate::cops::testing::correction_offenses(&HashSyntax { style }, source);
        let position = |o: &Offense| (o.line, o.col, o.message.clone());
        found.iter().map(position).collect()
    }

    #[test]
    fn hash_rockets_style_flags_and_corrects_every_colon_pair() {
        // RuboCop's examples for `EnforcedStyle: hash_rockets` (MIT).
        let rockets = HashSyntax {
            style: Style::HashRockets,
        };
        for (before, after) in [
            ("x = { a: 0, b: 2}\n", "x = { :a => 0, :b => 2}\n"),
            ("x = { a => 0, b: 1 }\n", "x = { a => 0, :b => 1 }\n"),
            ("func(3, a: 0)\n", "func(3, :a => 0)\n"),
            ("cc(\n  bb:\n)\n", "cc(\n  :bb => bb\n)\n"),
            ("f(a: a)\n", "f(:a => a)\n"),
            ("{ \"a b\": 1, c?:   2 }\n", "{ :\"a b\" => 1, :c? => 2 }\n"),
            // The value joins the line of its key, as in RuboCop.
            ("{ a:\n  1 }\n", "{ :a =>   1 }\n"),
            // A shorthand pair named after a keyword has no rocket form.
            (
                "def m(if:, bar:)\n  f(if:, bar:)\nend\n",
                "def m(if:, bar:)\n  f(if:, :bar => bar)\nend\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&rockets, before, after);
        }
        assert_eq!(
            styled(Style::HashRockets, "x = { a: 0, b: 2}\n"),
            [
                (1, 7, MSG_HASH_ROCKETS.to_string()),
                (1, 13, MSG_HASH_ROCKETS.to_string())
            ]
        );
        for source in ["x = { :a => 0, :b => 1 }\n", "func(3, :a => 0)\n", "{}\n"] {
            assert!(styled(Style::HashRockets, source).is_empty(), "{source}");
        }
    }

    #[test]
    fn no_mixed_keys_styles_flag_the_pairs_that_differ() {
        // RuboCop's examples for `no_mixed_keys` and `ruby19_no_mixed_keys`.
        use Style::{NoMixedKeys, Ruby19NoMixedKeys};
        let mixed = |col| vec![(1, col, MSG_NO_MIXED_KEYS.to_string())];
        for (style, source, expected) in [
            (NoMixedKeys, "x = { :a => 0, b: 1 }\n", mixed(16)),
            (NoMixedKeys, "{ a: 1, :b => 2 }\n", mixed(9)),
            (NoMixedKeys, "x = { a: 0, \"b\" => 1 }\n", mixed(7)),
            (NoMixedKeys, "{:\"t o\" => 0, b:}\n", mixed(15)),
            (NoMixedKeys, "x = { :a => 0 }\n", vec![]),
            (NoMixedKeys, "x = { a: 0, b: 1 }\n", vec![]),
            (NoMixedKeys, "x = { :a => 0, \"b\" => 1 }\n", vec![]),
            (
                Ruby19NoMixedKeys,
                "x = { :a => 0, b: 1 }\n",
                vec![(1, 7, MSG_19.to_string())],
            ),
            (Ruby19NoMixedKeys, "x = { a: 0, \"b\" => 1 }\n", mixed(7)),
            (Ruby19NoMixedKeys, "x = { :a => 0, \"b\" => 1 }\n", vec![]),
            (Ruby19NoMixedKeys, "x = { a: 0, b: 1 }\n", vec![]),
        ] {
            assert_eq!(styled(style, source), expected, "{source}");
        }
        for (style, before, after) in [
            (
                NoMixedKeys,
                "x = { :a => 0, b: 1 }\n",
                "x = { :a => 0, :b => 1 }\n",
            ),
            (NoMixedKeys, "{ a: 1, :b => 2 }\n", "{ a: 1, b: 2 }\n"),
            (
                NoMixedKeys,
                "{:\"t o\" => 0, b:}\n",
                "{:\"t o\" => 0, :b => b}\n",
            ),
            (
                Ruby19NoMixedKeys,
                "x = { :a => 0, b: 1 }\n",
                "x = { a: 0, b: 1 }\n",
            ),
            (
                Ruby19NoMixedKeys,
                "x = { a: 0, \"b\" => 1 }\n",
                "x = { :a => 0, \"b\" => 1 }\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&HashSyntax { style }, before, after);
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
                &HashSyntax::default(),
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
            crate::cops::testing::assert_correction(&HashSyntax::default(), before, after);
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
