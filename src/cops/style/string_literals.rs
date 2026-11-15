//! `Style/StringLiterals` — flags single-quoted strings that could be
//! double-quoted. Double quotes are rustocop's style; with
//! `enforced_style = "single_quotes"` the cop flags the double-quoted
//! strings that could be single-quoted.
//!
//! Matches RuboCop's semantics for `EnforcedStyle` with the default
//! `ConsistentQuotesInMultiline: false`. For double quotes, a `'…'` literal
//! is reported unless its source contains a `"`, a backslash escape other
//! than `\'` or `\\`, or text that would interpolate in double quotes (`#{`,
//! `#@`, `#$`). For single quotes, a `"…"` literal without interpolation is
//! reported unless its source contains a `'` or a backslash escape other
//! than `\"` or `\\`. Accepted as RuboCop does:
//!
//! - `%q()`, `%Q()`, `%()`, `?x` character literals, heredocs, and `%w`/`%i`
//!   elements;
//! - strings inside the interpolation of a string, symbol or regexp
//!   (`Style/StringLiteralsInInterpolation` checks those), but not inside a
//!   backtick command's interpolation;
//! - a literal spanning several lines (`'a⏎b'`): the parser gem splits it
//!   into one string per line, which RuboCop never checks.
//!
//! Each part of an adjacent-literal concatenation (`'a' 'b'`, `'a' \⏎ 'b'`)
//! is checked on its own. Offenses are reported at the opening quote.

use ruby_prism::{
    InterpolatedMatchLastLineNode, InterpolatedRegularExpressionNode, InterpolatedStringNode,
    InterpolatedSymbolNode, Node, NodeList, StringNode, Visit,
};

use crate::{
    cops::{
        Cop, NodeContext,
        settings::{Kind, Setting, Values},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

/// `enforced_style` for the cops that check the quotes of string literals.
pub(super) const ENFORCED_STYLE: Setting = Setting {
    name: "enforced_style",
    kind: Kind::Choice(&["double_quotes", "single_quotes"]),
};

/// The cop for the `enforced_style` in `values`: `single` for single quotes,
/// and `double` for any other style and when `values` name none.
pub(super) fn styled<S: Cop + 'static, D: Cop + 'static>(
    values: &Values,
    single: S,
    double: D,
) -> Option<Box<dyn Cop>> {
    Some(match values.choice("enforced_style") {
        Some("single_quotes") => Box::new(single),
        _ => Box::new(double),
    })
}

/// The `Style/StringLiterals` cop, for double quotes.
pub struct StringLiterals;

/// The cop for `enforced_style = "single_quotes"`.
struct SingleQuotes;

impl Cop for StringLiterals {
    fn name(&self) -> &'static str {
        "Style/StringLiterals"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn settings(&self) -> &'static [Setting] {
        &[ENFORCED_STYLE]
    }

    fn configure(&self, values: &Values) -> Option<Box<dyn Cop>> {
        styled(values, SingleQuotes, StringLiterals)
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        check(self, false, node, file, out);
    }
}

impl Cop for SingleQuotes {
    fn name(&self) -> &'static str {
        "Style/StringLiterals"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        check(self, true, node, file, out);
    }
}

/// Report the literals below the root `node` that have the other quotes
/// than the style asks for: single quotes when `single`, else double.
fn check(cop: &dyn Cop, single: bool, node: &Node<'_>, file: &SourceFile, out: &mut Vec<Offense>) {
    // Strings inside an interpolation are the other cop's business, so
    // one walk from the root skips those.
    let other: &[u8] = if single { b"\"" } else { b"'" };
    if node.as_program_node().is_none() || !file.contains(other) {
        return;
    }
    let message = if single {
        "Prefer single-quoted strings when you don't need string interpolation or special \
         symbols."
    } else {
        "Prefer double-quoted strings unless you need single quotes to avoid extra backslashes \
         for escaping."
    };
    for (offset, correction) in wrong_quotes(node, file, single) {
        let (line, col) = file.line_col(offset);
        out.push(Offense {
            correction,
            cop: cop.name(),
            severity: cop.severity(),
            message: message.to_string(),
            line,
            end_line: line,
            col,
        });
    }
}

/// Corrections for the literals at or below `node` that could have the
/// quotes of the style (single quotes when `single`, else double), not
/// counting those inside the interpolation of a string, symbol or regexp
/// (`"#{…}"`, `:"#{…}"`, `/#{…}/`) at or below it.
pub(super) fn wrong_quotes(
    node: &Node<'_>,
    file: &SourceFile,
    single: bool,
) -> Vec<(usize, Option<Correction>)> {
    let mut search = QuoteSearch {
        file,
        single,
        offsets: Vec::new(),
    };
    search.visit(node);
    search.offsets
}

/// Collects the strings with needless quotes of the other kind, skipping
/// the interpolations that RuboCop's `inside_interpolation?` covers.
struct QuoteSearch<'a> {
    file: &'a SourceFile,
    /// Whether the style is single quotes.
    single: bool,
    offsets: Vec<(usize, Option<Correction>)>,
}

impl QuoteSearch<'_> {
    /// Visit an interpolated literal's parts, except its interpolations.
    fn visit_literal_parts(&mut self, parts: NodeList<'_>) {
        for part in parts.iter() {
            if part.as_embedded_statements_node().is_none() {
                self.visit(&part);
            }
        }
    }
}

impl<'pr> Visit<'pr> for QuoteSearch<'_> {
    fn visit_string_node(&mut self, node: &StringNode<'pr>) {
        // The quote of the style, and the one that is reported.
        let (quote, other) = if self.single {
            (b'\'', b'"')
        } else {
            (b'"', b'\'')
        };
        let opening = node.opening_loc();
        if !opening.is_some_and(|opening| opening.as_slice() == [other]) {
            return;
        }
        let location = node.location();
        let source = location.as_slice();
        let single_line =
            self.file.line_of(location.start_offset()) == self.file.line_of(location.end_offset());
        let needs_its_quotes = if self.single {
            super::literal_spelling::double_quotes_required(source.iter().copied())
        } else {
            double_quotes_need_escaping(source)
        };
        if single_line && !needs_its_quotes {
            // The escapes left are `\\` and the escaped quote, which needs
            // no backslash between the other quotes.
            // For single quotes RuboCop writes a character that
            // `String#inspect` escapes, such as a raw tab, as its escape in
            // double quotes. The quotes change here and the value is the same.
            let mut replacement = vec![quote];
            let mut index = 1;
            while index < source.len() - 1 {
                if source[index] == b'\\' && index + 1 < source.len() - 1 {
                    if source[index + 1] != other {
                        replacement.push(b'\\');
                    }
                    index += 1;
                }
                replacement.push(source[index]);
                index += 1;
            }
            replacement.push(quote);
            // Correction stores UTF-8; never replace non-UTF-8 source lossily.
            let correction = String::from_utf8(replacement)
                .ok()
                .map(|replacement| Correction {
                    range: location.start_offset()..location.end_offset(),
                    replacement,
                });
            self.offsets.push((location.start_offset(), correction));
        }
    }

    fn visit_interpolated_string_node(&mut self, node: &InterpolatedStringNode<'pr>) {
        self.visit_literal_parts(node.parts());
    }

    fn visit_interpolated_symbol_node(&mut self, node: &InterpolatedSymbolNode<'pr>) {
        self.visit_literal_parts(node.parts());
    }

    fn visit_interpolated_regular_expression_node(
        &mut self,
        node: &InterpolatedRegularExpressionNode<'pr>,
    ) {
        self.visit_literal_parts(node.parts());
    }

    fn visit_interpolated_match_last_line_node(
        &mut self,
        node: &InterpolatedMatchLastLineNode<'pr>,
    ) {
        self.visit_literal_parts(node.parts());
    }
}

/// Whether a quoted literal's source needs its single quotes: it contains a
/// `"`, a backslash before anything but `'` or `\`, or `#{`, `#@` or `#$`
/// (RuboCop's `/" | \\[^'\\] | \#[@{$]/x`).
fn double_quotes_need_escaping(source: &[u8]) -> bool {
    source.iter().enumerate().any(|(index, &byte)| {
        let next = source.get(index + 1);
        match byte {
            b'"' => true,
            b'\\' => next.is_some_and(|next| *next != b'\'' && *next != b'\\'),
            b'#' => matches!(next, Some(b'@' | b'{' | b'$')),
            _ => false,
        }
    })
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/style/string_literals_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the
    //! `double_quotes` style.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/StringLiterals")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn corrects_quotes_and_escapes() {
        // RuboCop expect_correction examples, double_quotes style (MIT).
        for (before, after) in [
            ("s = 'abc'\n", "s = \"abc\"\n"),
            (r"'\''", r#""'""#),
            (r"'\\'", r#""\\""#),
            (
                r"'This \'string\' has \'multiple\' escaped quotes'",
                r#""This 'string' has 'multiple' escaped quotes""#,
            ),
            ("puts 'España'\r\n", "puts \"España\"\r\n"),
            ("'a' 'b'\n", "\"a\" \"b\"\n"),
        ] {
            assert!(StringLiterals.safe_fixes());
            let found = crate::cops::testing::correction_offenses(&StringLiterals, before);
            assert!(found.iter().all(|o| o.correction.is_some()), "{before}");
            assert_eq!(
                crate::corrections::apply(before.as_bytes(), &found)
                    .unwrap()
                    .bytes,
                after.as_bytes(),
                "{before}"
            );
        }
    }

    #[test]
    fn flags_single_quotes_with_message() {
        let found = offenses("s = 'abc'\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 5));
        assert_eq!(
            found[0].message,
            "Prefer double-quoted strings unless you need single quotes to avoid extra \
             backslashes for escaping."
        );
    }

    #[test]
    fn flags_rubocop_spec_offenses() {
        for (source, expected) in [
            ("s = \"abc\"\nx = 'abc'\n", vec![(2, 5)]),
            ("'\\''\n'\\\\'\n", vec![(1, 1), (2, 1)]),
            (
                "'This \\'string\\' has \\'multiple\\' escaped quotes'\n",
                vec![(1, 1)],
            ),
            ("a = 'blah #'\n", vec![(1, 5)]),
        ] {
            assert_eq!(positions(source), expected, "{source:?}");
        }
    }

    #[test]
    fn accepts_rubocop_spec_examples() {
        for source in [
            "a = \"x\"\n",
            "\"hello#{hash['there']}\"\n",
            "a = %q(x) + %Q[x]\n",
            "a = %(x)\n",
            "'\\,'\n'100\\%'\n'(\\)'\n",
            "execute <<-SQL\n  SELECT name from users\nSQL\n",
            "'\\n'\n",
            "'\\'\\n'\n",
            "a = '\\n'\nb = '\"'\nc = '#{x}'\nd = '#@x'\ne = '#$x'\nf = '\\s'\ng = '\\z'\n",
            "s = /'((?:[^\\']|\\.)*)'/\n",
            "%w(')\n",
            "if __FILE__ == $PROGRAM_NAME\nend\n",
            "a = ?/\n",
        ] {
            assert!(offenses(source).is_empty(), "should accept {source:?}");
        }
    }

    #[test]
    fn flags_other_single_quoted_strings() {
        for (source, expected) in [
            ("puts 'España'\n", vec![(1, 6)]),
            ("foo('a', 'b')\n", vec![(1, 5), (1, 10)]),
            ("'a' 'b'\n", vec![(1, 1), (1, 5)]),
            ("'a' \\\n  'b'\n", vec![(1, 1), (2, 3)]),
            ("\"#{x}\" 'b'\n", vec![(1, 8)]),
            ("'\\\\n'\n", vec![]),
            ("{ 'a' => 1 }\n", vec![(1, 3)]),
            ("`ls #{'a'}`\n", vec![(1, 7)]),
            ("x = '' + ' '\n", vec![(1, 5), (1, 10)]),
            ("def foo\n  'bar'\nend\n", vec![(2, 3)]),
        ] {
            assert_eq!(positions(source), expected, "{source:?}");
        }
    }

    #[test]
    fn accepts_lookalikes() {
        for source in [
            // Interpolations of strings, symbols, heredocs and regexps.
            "\"#{'A'}\"\n",
            ":\"#{'A'}\"\n",
            "<<~RUBY\n  #{'A'}\nRUBY\n",
            "/#{'A'}/\n",
            "if /#{'A'}/\nend\n",
            "\"#{foo('a', \"#{'b'}\")}\"\n",
            "%W[#{'a'}]\n",
            "/#{`#{'z'}`}/\n",
            // Literals RuboCop never checks.
            "x = 'a\nb'\n",
            "<<~'EOS'\n  text\nEOS\n",
            "%q('a')\n",
            "?'\n",
            ":'a'\n",
            "{ 'a': 1 }\n",
            "%w['a' b]\n",
            "%i[a b]\n",
            "x = \"it's\"\n",
        ] {
            assert!(offenses(source).is_empty(), "should accept {source:?}");
        }
    }

    #[test]
    fn single_quotes_style_flags_and_corrects_needless_double_quotes() {
        // RuboCop's examples for `EnforcedStyle: single_quotes` (MIT).
        let single = Values::default().with(
            "enforced_style",
            crate::cops::settings::Value::Choice("single_quotes"),
        );
        let cop = StringLiterals.configure(&single).unwrap();
        for (before, after) in [
            ("s = \"abc\"\n", "s = 'abc'\n"),
            (r#"x = "a\\b""#, r"x = 'a\\b'"),
            (r#"x = "say \"hi\"""#, r#"x = 'say "hi"'"#),
            (r#"x = "\\\"""#, r#"x = '\\"'"#),
            ("\"a\" \"b\"\n", "'a' 'b'\n"),
            ("foo(\"España\", 'b')\n", "foo('España', 'b')\n"),
        ] {
            crate::cops::testing::assert_correction(cop.as_ref(), before, after);
        }
        let found = crate::cops::testing::correction_offenses(cop.as_ref(), "s = \"abc\"\n");
        assert_eq!((found[0].line, found[0].col), (1, 5));
        assert_eq!(
            found[0].message,
            "Prefer single-quoted strings when you don't need string interpolation or special \
             symbols."
        );
        for source in [
            "a = 'x'\n",
            "a = \"it's\"\n",
            "a = \"\\n\"\nb = \"\\e[0m\"\nc = \"\\#{x}\"\n",
            "a = \"#{x}\"\nb = \"#@x\"\n",
            "a = %Q(x) + %(y)\n",
            "a = \"a\nb\"\n",
            // The other cop checks strings inside an interpolation.
            "\"#{\"A\"}\"\n",
            ":\"a\"\n",
            "{ \"a\": 1 }\n",
            "<<~\"EOS\"\n  text\nEOS\n",
            "?\"\n",
        ] {
            let found = crate::cops::testing::correction_offenses(cop.as_ref(), source);
            assert!(found.is_empty(), "should accept {source:?}");
        }
    }

    #[test]
    fn honors_directives() {
        assert!(offenses("x = 'a' # rubocop:disable Style/StringLiterals\n").is_empty());
    }
}
