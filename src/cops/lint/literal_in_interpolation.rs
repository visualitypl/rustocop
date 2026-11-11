//! `Lint/LiteralInInterpolation` — flags literals interpolated into strings,
//! symbols, backticks and regexps (`"result is #{10}"`), which can be
//! written inline.
//!
//! Matches RuboCop's semantics. The last statement of an interpolation is
//! reported when it prints as itself: a basic literal (string, symbol,
//! number, `true`/`false`/`nil`), or an array, hash or range made only of
//! such. Exceptions, as in RuboCop:
//!
//! - `__FILE__`, `__LINE__` and heredoc strings (special keywords).
//! - A whitespace-only string ending a heredoc line (it keeps trailing
//!   whitespace visible).
//! - An array in a regexp (`Lint/ArrayLiteralInRegexp`'s business).
//! - Inlining would let a preceding unescaped `#` start a new
//!   interpolation (`"##{nil}@foo"`).
//! - Inside `%W[]`/`%I[]`, a value that is empty or contains whitespace
//!   (inlining would split or drop a word).
//!
//! Endless and beginless ranges crash RuboCop's cop, so they are never
//! reported here. The offense is reported at the literal.

use ruby_prism::{EmbeddedStatementsNode, Node, NodeList};

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/LiteralInInterpolation` cop.
pub struct LiteralInInterpolation;

impl Cop for LiteralInInterpolation {
    fn name(&self) -> &'static str {
        "Lint/LiteralInInterpolation"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (parts, context) = match node {
            Node::InterpolatedStringNode { .. } => {
                let string = node.as_interpolated_string_node().unwrap();
                let opening = string.opening_loc();
                let opening = opening.as_ref().map(|opening| opening.as_slice());
                let parts = string.parts();
                let context = Context {
                    heredoc: opening.is_some_and(|opening| opening.starts_with(b"<<")),
                    in_regexp: false,
                    // `%W[]` elements have no delimiters of their own; the
                    // other delimiter-less strings are concatenations
                    // (`"a" "b"`), whose parts are strings.
                    word: opening.is_none() && has_interpolation(&parts),
                };
                (parts, context)
            }
            Node::InterpolatedSymbolNode { .. } => {
                let symbol = node.as_interpolated_symbol_node().unwrap();
                let context = Context {
                    word: symbol.opening_loc().is_none(),
                    ..Context::default()
                };
                (symbol.parts(), context)
            }
            Node::InterpolatedXStringNode { .. } => (
                node.as_interpolated_x_string_node().unwrap().parts(),
                Context::default(),
            ),
            Node::InterpolatedRegularExpressionNode { .. } => (
                node.as_interpolated_regular_expression_node()
                    .unwrap()
                    .parts(),
                Context::regexp(),
            ),
            Node::InterpolatedMatchLastLineNode { .. } => (
                node.as_interpolated_match_last_line_node().unwrap().parts(),
                Context::regexp(),
            ),
            _ => return,
        };
        for part in parts.iter() {
            if let Some(interpolation) = part.as_embedded_statements_node()
                && let Some(literal) = offending_literal(&interpolation, &context, file)
            {
                let (line, col) = super::array_literal_in_regexp::reference_line_col(
                    literal.start_offset(),
                    file,
                );
                out.push(Offense {
                    correction: None,
                    cop: self.name(),
                    severity: self.severity(),
                    message: "Literal interpolation detected.".to_string(),
                    line,
                    end_line: file.last_line_of(literal),
                    col,
                });
            }
        }
    }
}

/// Where the interpolations being checked sit.
#[derive(Default)]
struct Context {
    /// In a heredoc.
    heredoc: bool,
    /// In a regexp.
    in_regexp: bool,
    /// In a word of a `%W[]` or `%I[]` literal.
    word: bool,
}

impl Context {
    fn regexp() -> Self {
        Context {
            in_regexp: true,
            ..Context::default()
        }
    }
}

fn has_interpolation(parts: &NodeList<'_>) -> bool {
    parts.iter().any(|part| {
        part.as_embedded_statements_node().is_some() || part.as_embedded_variable_node().is_some()
    })
}

/// The source location of the literal to report in `interpolation`, if any.
fn offending_literal<'pr>(
    interpolation: &EmbeddedStatementsNode<'pr>,
    context: &Context,
    file: &SourceFile,
) -> Option<ruby_prism::Location<'pr>> {
    let last = interpolation.statements()?.body().iter().last()?;
    if is_special_keyword(&last) || !prints_as_self(&last, file) {
        return None;
    }
    if context.heredoc && is_space_literal(&last, file) && ends_line(interpolation, &last, file) {
        return None;
    }
    if context.in_regexp && last.as_array_node().is_some() {
        return None;
    }
    let value = Value::of(&last);
    if starts_interpolation_after_hash_sign(interpolation, &value, file) {
        return None;
    }
    if context.word && (value.first.is_none() || value.whitespace) {
        return None;
    }
    Some(last.location())
}

/// `__FILE__`, `__LINE__` and heredocs (RuboCop: strings without an opening
/// delimiter, or `__LINE__`).
fn is_special_keyword(node: &Node<'_>) -> bool {
    match node {
        Node::SourceFileNode { .. } | Node::SourceLineNode { .. } => true,
        Node::StringNode { .. } => node
            .as_string_node()
            .unwrap()
            .opening_loc()
            .is_none_or(|opening| opening.as_slice().starts_with(b"<<")),
        Node::InterpolatedStringNode { .. } => node
            .as_interpolated_string_node()
            .unwrap()
            .opening_loc()
            .is_some_and(|opening| opening.as_slice().starts_with(b"<<")),
        _ => false,
    }
}

/// Whether the node prints as its own source: basic literals, and arrays,
/// hashes and ranges of such.
fn prints_as_self(node: &Node<'_>, file: &SourceFile) -> bool {
    if super::reference_basic_literal::is_basic_literal(node, file) {
        return true;
    }
    match node {
        Node::ArrayNode { .. } => node
            .as_array_node()
            .unwrap()
            .elements()
            .iter()
            .all(|element| prints_as_self(&element, file)),
        Node::HashNode { .. } => {
            pairs_print_as_self(&node.as_hash_node().unwrap().elements(), file)
        }
        // `[a: 1]`: a plain `hash` in RuboCop's AST. An array's value is
        // inlined as its source, so `Value::of` needs no counterpart.
        Node::KeywordHashNode { .. } => {
            pairs_print_as_self(&node.as_keyword_hash_node().unwrap().elements(), file)
        }
        // RuboCop crashes on endless and beginless ranges, reporting nothing.
        Node::RangeNode { .. } => {
            let range = node.as_range_node().unwrap();
            range.left().is_some_and(|left| prints_as_self(&left, file))
                && range
                    .right()
                    .is_some_and(|right| prints_as_self(&right, file))
        }
        _ => false,
    }
}

fn pairs_print_as_self(elements: &NodeList<'_>, file: &SourceFile) -> bool {
    elements.iter().all(|element| {
        element.as_assoc_node().is_some_and(|pair| {
            prints_as_self(&pair.key(), file) && prints_as_self(&pair.value(), file)
        })
    })
}

/// A string of (Unicode) whitespace only, possibly empty.
fn is_space_literal(node: &Node<'_>, file: &SourceFile) -> bool {
    node.as_string_node().is_some_and(|string| {
        let single = string.opening_loc().is_some_and(|opening| {
            opening.as_slice().starts_with(b"'") || opening.as_slice().starts_with(b"%q")
        });
        // The reference transcodes source characters before parsing escapes.
        // Align with the main parse's cooked value so numeric/Unicode escape
        // results keep their bytes instead of being transcoded a second time.
        let value = crate::cops::ast::require_literal::parts(
            string.content_loc().as_slice(),
            string.unescaped(),
            single,
            file,
        )
        .concat();
        std::str::from_utf8(&value).is_ok_and(|text| text.chars().all(char::is_whitespace))
    })
}

/// RuboCop's `ends_heredoc_line?`: the interpolation ends where the line
/// of the literal's end does.
fn ends_line(
    interpolation: &EmbeddedStatementsNode<'_>,
    last: &Node<'_>,
    file: &SourceFile,
) -> bool {
    let last_line = file.line_of(last.location().end_offset().saturating_sub(1));
    let projected = file.match_pattern_source(file.line_text(last_line));
    let line_length = projected
        .utf8_chunks()
        .map(|c| c.valid().chars().count() + c.invalid().len())
        .sum::<usize>() as u32;
    let (_, end_col) = super::array_literal_in_regexp::reference_line_col(
        interpolation.location().end_offset(),
        file,
    );
    line_length == end_col - 1
}

/// RuboCop's `starts_interpolation_after_hash_sign?`: inlining the value
/// after an unescaped `#` would make `#{`, `#@` or `#$`.
fn starts_interpolation_after_hash_sign(
    interpolation: &EmbeddedStatementsNode<'_>,
    value: &Value,
    file: &SourceFile,
) -> bool {
    let bytes = &file.bytes;
    let location = interpolation.location();
    let (start, end) = (location.start_offset(), location.end_offset());
    let following = value.first.or_else(|| bytes.get(end).copied());
    if !matches!(following, Some(b'{' | b'@' | b'$')) {
        return false;
    }
    if start == 0 || bytes[start - 1] != b'#' {
        return false;
    }
    let backslashes = bytes[..start - 1]
        .iter()
        .rev()
        .take_while(|&&byte| byte == b'\\')
        .count();
    backslashes % 2 == 0
}

/// What RuboCop's autocorrection would inline, as far as the checks need it.
struct Value {
    /// Its first byte; `None` when empty.
    first: Option<u8>,
    /// Whether it contains ASCII whitespace (Ruby's `\s`).
    whitespace: bool,
}

impl Value {
    fn of(node: &Node<'_>) -> Self {
        match node {
            Node::NilNode { .. } => Value::text(b""),
            // Decimal digits, maybe signed: never whitespace or `{@$`.
            Node::IntegerNode { .. } | Node::FloatNode { .. } => Value::text(b"0"),
            Node::StringNode { .. } => {
                let string = node.as_string_node().unwrap();
                let unescaped = string.unescaped();
                if std::str::from_utf8(unescaped).is_ok() {
                    Value::text(unescaped)
                } else {
                    // RuboCop inlines the source without its quotes.
                    let source = node.location().as_slice();
                    let source = source.strip_prefix(b"\"").unwrap_or(source);
                    Value::text(source.strip_suffix(b"\"").unwrap_or(source))
                }
            }
            Node::SymbolNode { .. } => {
                // The source between the delimiters, escapes and all.
                let symbol = node.as_symbol_node().unwrap();
                let location = node.location();
                let start = symbol
                    .opening_loc()
                    .map_or(location.start_offset(), |opening| opening.end_offset());
                let end = symbol
                    .closing_loc()
                    .map_or(location.end_offset(), |closing| closing.start_offset());
                let source = location.as_slice();
                let base = location.start_offset();
                Value::text(&source[start - base..end.max(start) - base])
            }
            Node::ArrayNode { .. } => {
                let array = node.as_array_node().unwrap();
                let percent = array
                    .opening_loc()
                    .is_some_and(|opening| opening.as_slice().starts_with(b"%"));
                if percent {
                    // Upstream splits the original percent contents, not the
                    // parsed elements: even an escaped space splits two words.
                    let location = node.location();
                    let start = array.opening_loc().unwrap().end_offset();
                    let end = array.closing_loc().unwrap().start_offset();
                    let contents = &location.as_slice()
                        [start - location.start_offset()..end - location.start_offset()];
                    Value {
                        first: Some(b'['),
                        whitespace: contents
                            .split(|&b| is_ruby_space(b))
                            .filter(|part| !part.is_empty())
                            .nth(1)
                            .is_some(),
                    }
                } else {
                    Value::text(node.location().as_slice())
                }
            }
            Node::HashNode { .. } => {
                let elements = node.as_hash_node().unwrap().elements();
                Value {
                    first: Some(b'{'),
                    whitespace: elements.iter().nth(1).is_some()
                        || elements.iter().any(|pair| {
                            pair.as_assoc_node().is_some_and(|pair| {
                                in_hash_has_whitespace(&pair.key())
                                    || in_hash_has_whitespace(&pair.value())
                            })
                        }),
                }
            }
            _ => Value::text(node.location().as_slice()),
        }
    }

    fn text(text: &[u8]) -> Self {
        Value {
            first: text.first().copied(),
            whitespace: text.iter().any(|byte| is_ruby_space(*byte)),
        }
    }
}

/// Whether RuboCop's rendering of a hash key or value (`inspect` for
/// strings and symbols) contains whitespace.
fn in_hash_has_whitespace(node: &Node<'_>) -> bool {
    match node {
        Node::IntegerNode { .. } | Node::FloatNode { .. } => false,
        // `inspect` escapes control characters, so only spaces remain.
        Node::StringNode { .. } => node.as_string_node().unwrap().unescaped().contains(&b' '),
        Node::SymbolNode { .. } => node.as_symbol_node().unwrap().unescaped().contains(&b' '),
        _ => Value::of(node).whitespace,
    }
}

/// Ruby's `\s`.
fn is_ruby_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/literal_in_interpolation_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        let result = ruby_prism::parse(source.as_bytes());
        assert_eq!(result.errors().count(), 0, "fixture must parse: {source:?}");
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/LiteralInInterpolation")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    /// Columns of the offenses on the first line.
    fn cols(source: &str) -> Vec<u32> {
        positions(source)
            .into_iter()
            .map(|(line, col)| {
                assert_eq!(line, 1, "offense on line 1 of {source:?}");
                col
            })
            .collect()
    }

    /// RuboCop's `literal interpolation` shared examples.
    const LITERALS: &[&str] = &[
        "1",
        "-1",
        "1_123",
        "123_456_789_123_456_789",
        "0xaabb",
        "0o377",
        "1.2e-3",
        "2.0",
        "\"double_quot_string\"",
        "'single_quot_string'",
        "\"double_quot_string: '\"",
        "'single_quot_string: \"'",
        ":symbol",
        ":\"symbol\"",
        ":\"single quot in symbol: '\"",
        ":'double quot in symbol: \"'",
        "[]",
        "[\"a\", \"b\"]",
        "%w[]",
        "%w[v1]",
        "%w[v1 v2]",
        "%i[s1 s2]",
        "%I[s1 s2]",
        "%i[s1     s2]",
        "%i[ s1   s2 ]",
        "{\"a\" => \"b\"}",
        "{ foo: 'bar', :fiz => \"buzz\" }",
        "{ foo: { fiz: 'buzz' } }",
        "{ num: { separate: 1_123, long_separate: 123_456_789_123_456_789, exponent: 1.2e-3 } }",
        "{ n_adic_num: { hex: 0xaabb, oct: 0o377 } }",
        "{ double_quot: { simple: \"double_quot\", single_in_double: \"double_quot: '\" } }",
        "{ single_quot: { simple: 'single_quot', double_in_single: 'single_quot: \"' } }",
        "{ bool: { key: true } }",
        "{ bool: { key: false } }",
        "{ nil: { key: nil } }",
        "{ symbol: { key: :symbol } }",
        "{ symbol: { key: :\"symbol\" } }",
        "{ :'foo-bar' => 1 }",
        "{ symbol: { key: :\"foo-bar\" } }",
        "{ single_quot_symbol: { key: :\"single_quot_in_symbol: '\" } }",
        "{ double_quot_symbol: { key: :'double_quot_in_symbol: \"' } }",
        "{ single_quot_symbol_not_in_space: { key: :\"single_quot_in_symbol:'\" } }",
        "{ single_quot_symbol_in_space: { key: :\"single_quot_in_symbol: \" } }",
        "{ range: { key: 1..2 } }",
        "{ range: { key: 1...2 } }",
        "{ array: { key: %w[] } }",
        "{ array: { key: %w[v1] } }",
        "{ array: { key: %w[v1 v2] } }",
        "{ array: { key: %i[s1 s2] } }",
        "{ array: { key: %I[s1 s2] } }",
        "{ array: { key: %i[s1     s2] } }",
        "{ array: { key: %i[ s1   s2 ] } }",
        "nil",
        "1..2",
        "1...2",
        "true",
        "false",
    ];

    #[test]
    fn percent_array_significance_uses_source_words_even_for_escaped_spaces() {
        for prefix in ["%W", "%I"] {
            for literal in ["%w[a\\ b]", "%i[a\\ b]", "%W[a\\ b]", "%I[a\\ b]"] {
                let source = format!("{prefix}[#{{{literal}}}]");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        assert_eq!(cols("%W[#{%w[a]}]"), [6]);
    }

    #[test]
    fn static_prism_literals_must_be_reference_basic_literals_recursively() {
        for literal in [
            "%q()",
            "%Q()",
            "\"a\nb\"",
            "'a\nb'",
            ":\"\"",
            "%s()",
            ":\"a\nb\"",
            "\"a\\n\\\n\"",
        ] {
            for value in [
                literal.to_string(),
                format!("[1, [[{literal}]]]"),
                format!("{{x: {literal}}}"),
                format!("{{{literal} => 1}}"),
                format!("{literal}..1"),
            ] {
                let source = format!("\"#{{{value}}}\"\n");
                assert!(offenses(&source).is_empty(), "{source:?}");
            }
        }
        for literal in ["\"\"", "''", "\"\\u{}\"", ":\"\\u{}\"", ":x", "\"a\\\nb\""] {
            assert_eq!(
                positions(&format!("\"#{{{literal}}}\"")),
                [(1, 4)],
                "{literal:?}"
            );
        }
        // Quoted hash labels bypass symbol composition, even empty/multiline.
        assert_eq!(positions("\"#{{\"\": 1}}\""), [(1, 4)]);
    }

    #[test]
    fn flags_literal_with_message() {
        let found = offenses("\"this is the #{1}\"\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 16));
        assert_eq!(found[0].message, "Literal interpolation detected.");
    }

    #[test]
    fn flags_every_literal_of_the_rubocop_shared_examples() {
        for literal in LITERALS {
            let len = literal.chars().count() as u32;
            let cases: [(String, Vec<u32>); 6] = [
                (format!("\"this is the #{{{literal}}}\"\n"), vec![16]),
                (
                    format!("\"this is the #{{{literal}}} literally\"\n"),
                    vec![16],
                ),
                (
                    format!("\"some #{{{literal}}} with #{{{literal}}} too\"\n"),
                    vec![9, len + 18],
                ),
                (
                    format!("\"this is #{{{literal}}} with #{{a}} now\"\n"),
                    vec![12],
                ),
                (
                    format!("\"this is #{{a}} with #{{{literal}}} now\"\n"),
                    vec![22],
                ),
                (
                    format!("\"this is the #{{{literal};{literal}}}\"\n"),
                    vec![17 + len],
                ),
            ];
            for (source, expected) in cases {
                assert_eq!(cols(&source), expected, "for {source:?}");
            }
        }
    }

    #[test]
    fn accepts_non_literal_interpolation() {
        for source in [
            "\"this is #{a} silly\"\n",
            "\"this is #{`a`} silly\"\n",
            "\"this is an irange: #{var1..var2}\"\n",
            "\"this is an erange: #{var1...var2}\"\n",
            "\"#{}\"\n",
            "\"#{(1)}\"\n",
            "\"#{\"#{a}\"}\"\n",
            "\"#{[a]}\"\n",
            "\"#{{ a => 1 }}\"\n",
            "\"#{{ a: }}\"\n",
            "\"#{[*a]}\"\n",
            "\"#{[a: b]}\"\n",
            "\"#{[**a]}\"\n",
            "\"#{/a/}\"\n",
            "\"#{1..}\"\n",
            "\"#{..1}\"\n",
            "\"#{__ENCODING__}\"\n",
            "\"#@foo #{@bar}\"\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn words_literals_keep_significant_interpolation() {
        for prefix in ["%W", "%I"] {
            for literal in [
                "\"this interpolation\"",
                "\"\"",
                ":\"this interpolation\"",
                "[\"this interpolation\"]",
                "[:\"this interpolation\"]",
            ] {
                let source = format!("{prefix}[#{{{literal}}} is significant]\n");
                assert!(offenses(&source).is_empty(), "should not flag {source:?}");
            }
            for literal in [
                "\"interpolation\"",
                ":interpolation",
                "[\"interpolation\"]",
                "[:interpolation]",
            ] {
                let source = format!("{prefix}[this #{{{literal}}} is not significant]\n");
                assert_eq!(cols(&source), [11], "for {source:?}");
            }
        }
        assert!(offenses("%W[a#{nil}]\n").is_empty());
        assert!(offenses("%W[#{%w[a b]}]\n").is_empty());
        assert_eq!(cols("%W[#{%w[a]} b]\n"), [6]);
        assert_eq!(cols("%I[a#{1}]\n"), [7]);
    }

    #[test]
    fn flags_arrays_holding_a_braceless_hash() {
        // `[a: 1]` holds a plain `hash` in RuboCop's AST.
        assert_eq!(cols("p \"#{[a: 1]}\"\n"), [6]);
        assert_eq!(cols("p \"#{[1, a: 2]}\"\n"), [6]);
        assert_eq!(cols("%W[#{[a:1]} b]\n"), [6]);
        assert!(offenses("%W[#{[a: 1]} b]\n").is_empty());
    }

    #[test]
    fn flags_nested_interpolation() {
        assert_eq!(cols("\"this is #{\"#{1}\"} silly\"\n"), [15]);
    }

    #[test]
    fn special_keywords() {
        for keyword in ["__FILE__", "__LINE__", "__END__", "__ENCODING__"] {
            let source = format!("%(\"this is #{{{keyword}}} silly\")\n");
            assert!(offenses(&source).is_empty(), "should not flag {source:?}");
            let source = format!("\"this is the #{{{keyword}}} #{{1}}\"\n");
            let col = 20 + keyword.len() as u32;
            assert_eq!(cols(&source), [col], "for {source:?}");
        }
        assert!(offenses("x = \"#{<<~A}\"\n  a\nA\n").is_empty());
        assert_eq!(cols("\"#{[__FILE__]}\"\n"), [4]);
    }

    #[test]
    fn flags_strings_with_quotes_and_escapes() {
        for source in ["\"this is the #{'foo'}\"\n", "\"this is the #{\"foo\"}\"\n"] {
            assert_eq!(cols(source), [16], "for {source:?}");
        }
        for source in [
            "\"this is #{'\"'} silly\"\n",
            "\"this is #{\"\\\"\"} silly\"\n",
            "\"this is #{\"\\#{1}\"} silly\"\n",
            "\"this is #{'\\n'} silly\"\n",
            "\"this is #{%q(\\n)} silly\"\n",
            "\"this is #{\"\\n\"} silly\"\n",
            "\"this is #{%(\\n)} silly\"\n",
            "\"this is #{%Q(\\n)} silly\"\n",
        ] {
            assert_eq!(cols(source), [12], "for {source:?}");
        }
        assert_eq!(
            cols("x = \"ABC\".gsub(/(A)(B)(C)/, \"D#{'\\2'}F\")\n"),
            [33]
        );
        // Invalid UTF-8 escapes.
        assert_eq!(cols("\"#{\"\\201\\203\"}\"\n"), [4]);
    }

    #[test]
    fn hash_signs_that_would_start_an_interpolation() {
        assert_eq!(cols("/^(#{'#'}{1,6})\\s/\n"), [6]);
        assert_eq!(cols("\"#{'#'}@foo\"\n"), [4]);
        for source in [
            "\"##{'{'}foo}\"\n",
            "\"##{''}{foo}\"\n",
            "\"##{nil}@foo\"\n",
            "/##{nil}{2}/\n",
            "\"##{:$a}\"\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
        assert_eq!(cols("\"\\##{''}{foo}\"\n"), [6]);
        assert_eq!(cols("\"\\\\##{''}{foo}\"\n"), Vec::<u32>::new());
        assert_eq!(cols("\"#{'#'}#{'{'}foo}\"\n"), [4, 10]);
        assert_eq!(cols("\"#{'#'}#{''}{foo}\"\n"), [4, 10]);
        assert_eq!(cols("\"#{1}#{2}\"\n"), [4, 8]);
    }

    #[test]
    fn whitespace_ending_a_heredoc_line_is_kept() {
        for source in [
            "<<~HERE\n  Line with explicit space literal at the end. #{'  '}\nHERE\n",
            "<<~HERE\n  Line with explicit space literal at the end. #{ ' ' }\nHERE\n",
            "<<~HERE\n  Line with explicit ideographic space literal at the end. #{'\u{3000}'}\nHERE\n",
            "x = <<-E\n  line #{\"  \"}\nE\n",
            "x = <<E\nline #{\"\"}\nE\n",
            "x = <<~E\n  #{' '}\nE\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
        assert_eq!(
            positions("x = <<~E\n  a\n  b #{' '}\n  c #{' '} d\nE\n"),
            [(4, 7)]
        );
        // Not a heredoc: reported.
        assert_eq!(cols("\"line #{' '}\"\n"), [9]);
    }

    #[test]
    fn heredoc_whitespace_projects_source_characters_not_escaped_bytes() {
        for literal in [
            r#""\xa0""#,
            r#""\x85""#,
            r#""\240""#,
            r#""\205""#,
            r#"%Q(\xa0)"#,
            r#"%q(\xa0)"#,
            r#"" \xa0""#,
        ] {
            let source = format!("# coding: ISO-8859-1\nx = <<~X\n  #{{{literal}}}\nX\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!((found[0].line, found[0].col), (3, 5));
            assert_eq!(found[0].message, "Literal interpolation detected.");
        }
        for literal in [r#""\xc2\xa0""#, r#""\u00a0""#, r#""\t ""#] {
            let source = format!("# coding: ISO-8859-1\nx = <<~X\n  #{{{literal}}}\nX\n");
            assert!(offenses(&source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn flags_in_string_like_contexts() {
        assert_eq!(cols(":\"this is the #{42}\"\n"), [17]);
        assert_eq!(cols("`this is the #{42}`\n"), [16]);
        assert_eq!(cols("/this is the #{42}/\n"), [16]);
        assert_eq!(cols("x if /a#{42}/\n"), [10]);
    }

    #[test]
    fn handles_regexp_special_characters() {
        for (source, col) in [
            ("/test#{'/'}test/\n", 8),
            ("/test#{'/a/b/c/'}test/\n", 8),
            ("/test#{'\\\\/'}test/\n", 8),
            ("/test#{'\\\\\\\\/'}test/\n", 8),
            ("%r{test#{'/'}test}\n", 10),
            ("\"test#{'/'}test\"\n", 8),
        ] {
            assert_eq!(cols(source), [col], "for {source:?}");
        }
    }

    #[test]
    fn arrays_in_regexps_are_left_to_another_cop() {
        assert!(offenses("/#{%w[a b c]}/\n").is_empty());
        assert!(offenses("x if /#{[1]}/\n").is_empty());
        assert_eq!(cols("/#{{ a: 1 }}/\n"), [4]);
    }

    #[test]
    fn honors_directives() {
        assert!(offenses("\"#{1}\" # rubocop:disable Lint/LiteralInInterpolation\n").is_empty());
    }
}
