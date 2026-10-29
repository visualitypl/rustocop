//! `Style/StringLiterals` — flags single-quoted strings that could be
//! double-quoted. rustocop's fixed style is double quotes.
//!
//! Matches RuboCop's semantics for `EnforcedStyle: double_quotes` with the
//! default `ConsistentQuotesInMultiline: false`. A `'…'` literal is reported
//! unless its source contains a `"`, a backslash escape other than `\'` or
//! `\\`, or text that would interpolate in double quotes (`#{`, `#@`, `#$`).
//! Accepted as RuboCop does:
//!
//! - `%q()`, `%Q()`, `%()`, `?x` character literals, heredocs, and `%w`/`%i`
//!   elements;
//! - strings inside the interpolation of a string, symbol or regexp
//!   (`Style/StringLiteralsInInterpolation` checks those), but not inside a
//!   backtick command's interpolation;
//! - a single-quoted literal spanning several lines (`'a⏎b'`): the parser
//!   gem splits it into one string per line, which RuboCop never checks.
//!
//! Each part of an adjacent-literal concatenation (`'a' 'b'`, `'a' \⏎ 'b'`)
//! is checked on its own. Offenses are reported at the opening quote.

use ruby_prism::{
    InterpolatedMatchLastLineNode, InterpolatedRegularExpressionNode, InterpolatedStringNode,
    InterpolatedSymbolNode, Node, NodeList, StringNode, Visit,
};

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

/// The `Style/StringLiterals` cop.
pub struct StringLiterals;

impl Cop for StringLiterals {
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
        // Strings inside an interpolation are the other cop's business, so
        // one walk from the root skips those.
        if node.as_program_node().is_none() || !file.contains(b"'") {
            return;
        }
        for (offset, correction) in needless_single_quotes(node, file) {
            let (line, col) = file.line_col(offset);
            out.push(Offense {
                correction,
                cop: self.name(),
                severity: self.severity(),
                message: "Prefer double-quoted strings unless you need single quotes to avoid \
                          extra backslashes for escaping."
                    .to_string(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

/// Corrections for the single-quoted literals at or below `node` that
/// could be double-quoted, not counting those inside the interpolation of a
/// string, symbol or regexp (`"#{…}"`, `:"#{…}"`, `/#{…}/`) at or below it.
pub(super) fn needless_single_quotes(
    node: &Node<'_>,
    file: &SourceFile,
) -> Vec<(usize, Option<Correction>)> {
    let mut search = SingleQuoteSearch {
        file,
        offsets: Vec::new(),
    };
    search.visit(node);
    search.offsets
}

/// Collects needlessly single-quoted strings, skipping the interpolations
/// that RuboCop's `inside_interpolation?` covers.
struct SingleQuoteSearch<'a> {
    file: &'a SourceFile,
    offsets: Vec<(usize, Option<Correction>)>,
}

impl SingleQuoteSearch<'_> {
    /// Visit an interpolated literal's parts, except its interpolations.
    fn visit_literal_parts(&mut self, parts: NodeList<'_>) {
        for part in parts.iter() {
            if part.as_embedded_statements_node().is_none() {
                self.visit(&part);
            }
        }
    }
}

impl<'pr> Visit<'pr> for SingleQuoteSearch<'_> {
    fn visit_string_node(&mut self, node: &StringNode<'pr>) {
        let single_quoted = node
            .opening_loc()
            .is_some_and(|opening| opening.as_slice() == b"'");
        if !single_quoted {
            return;
        }
        let location = node.location();
        let single_line =
            self.file.line_of(location.start_offset()) == self.file.line_of(location.end_offset());
        if single_line && !double_quotes_need_escaping(location.as_slice()) {
            let source = location.as_slice();
            let mut replacement = vec![b'"'];
            let mut index = 1;
            while index < source.len() - 1 {
                if source[index] == b'\\' && index + 1 < source.len() - 1 {
                    if source[index + 1] != b'\'' {
                        replacement.push(b'\\');
                    }
                    index += 1;
                }
                replacement.push(source[index]);
                index += 1;
            }
            replacement.push(b'"');
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
    fn honors_directives() {
        assert!(offenses("x = 'a' # rubocop:disable Style/StringLiterals\n").is_empty());
    }
}
