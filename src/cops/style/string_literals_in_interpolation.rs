//! `Style/StringLiteralsInInterpolation` — flags single-quoted strings
//! inside an interpolation that could be double-quoted. Double quotes are
//! rustocop's style; with `enforced_style = "single_quotes"` the cop flags
//! the double-quoted strings that could be single-quoted.
//!
//! Matches RuboCop's semantics for `EnforcedStyle`: the
//! strings checked are those inside the interpolation (`#{…}`) of a string,
//! heredoc, symbol or regexp, at any depth, including inside a backtick
//! command nested in one; a backtick command's own interpolation is not
//! covered (`Style/StringLiterals` checks it). A string qualifies on the
//! same terms as in `Style/StringLiterals`. Offenses are reported at the
//! opening quote.

use ruby_prism::Node;

use super::string_literals::{ENFORCED_STYLE, styled, wrong_quotes};
use crate::{
    cops::{
        Cop, NodeContext,
        settings::{Setting, Values},
    },
    offense::Offense,
    source::SourceFile,
};

/// The `Style/StringLiteralsInInterpolation` cop, for double quotes.
pub struct StringLiteralsInInterpolation;

/// The cop for `enforced_style = "single_quotes"`.
struct SingleQuotes;

impl Cop for StringLiteralsInInterpolation {
    fn name(&self) -> &'static str {
        "Style/StringLiteralsInInterpolation"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn settings(&self) -> &'static [Setting] {
        &[ENFORCED_STYLE]
    }

    fn configure(&self, values: &Values) -> Option<Box<dyn Cop>> {
        styled(values, SingleQuotes, StringLiteralsInInterpolation)
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
        "Style/StringLiteralsInInterpolation"
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

/// Report the strings inside the interpolations of `node` that have the
/// other quotes than the style asks for: single quotes when `single`.
fn check(cop: &dyn Cop, single: bool, node: &Node<'_>, file: &SourceFile, out: &mut Vec<Offense>) {
    let parts = match node {
        Node::InterpolatedStringNode { .. } => node.as_interpolated_string_node().unwrap().parts(),
        Node::InterpolatedSymbolNode { .. } => node.as_interpolated_symbol_node().unwrap().parts(),
        Node::InterpolatedRegularExpressionNode { .. } => node
            .as_interpolated_regular_expression_node()
            .unwrap()
            .parts(),
        Node::InterpolatedMatchLastLineNode { .. } => {
            node.as_interpolated_match_last_line_node().unwrap().parts()
        }
        _ => return,
    };
    // Each interpolation is searched down to (not into) the
    // interpolations of the literals nested in it, which get their own
    // call: every string is reported once.
    for part in parts.iter() {
        let Some(embedded) = part.as_embedded_statements_node() else {
            continue;
        };
        let Some(statements) = embedded.statements() else {
            continue;
        };
        for (offset, correction) in wrong_quotes(&statements.as_node(), file, single) {
            let (line, col) = file.line_col(offset);
            let kind = if single { "single" } else { "double" };
            out.push(Offense {
                correction,
                cop: cop.name(),
                severity: cop.severity(),
                message: format!("Prefer {kind}-quoted strings inside interpolations."),
                line,
                end_line: line,
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/style/string_literals_in_interpolation_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the
    //! `double_quotes` style.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/StringLiteralsInInterpolation")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn single_quotes_style_flags_double_quotes_inside_interpolations() {
        let single = Values::default().with(
            "enforced_style",
            crate::cops::settings::Value::Choice("single_quotes"),
        );
        let cop = StringLiteralsInInterpolation.configure(&single).unwrap();
        for (before, after) in [
            (r##""#{"A"}""##, r##""#{'A'}""##),
            (
                r##"/foo#{"sar".sub("s", 'b')}/"##,
                r##"/foo#{'sar'.sub('s', 'b')}/"##,
            ),
        ] {
            crate::cops::testing::assert_correction(cop.as_ref(), before, after);
        }
        let found = crate::cops::testing::correction_offenses(cop.as_ref(), "\"#{\"A\"}\"\n");
        assert_eq!(
            found[0].message,
            "Prefer single-quoted strings inside interpolations."
        );
        // Strings outside an interpolation are the other cop's.
        let found = crate::cops::testing::correction_offenses(cop.as_ref(), "x = \"a\"\n");
        assert!(found.is_empty());
    }

    #[test]
    fn corrects_interpolation_strings() {
        // RuboCop expect_correction examples (MIT), plus nested interpolation.
        for (before, after) in [
            (r##""#{'A'}""##, r##""#{"A"}""##),
            (
                r##"/foo#{'sar'.sub('s', 'b')}/"##,
                r##"/foo#{"sar".sub("s", "b")}/"##,
            ),
            (r##""#{"#{'a'}"}""##, r##""#{"#{"a"}"}""##),
            (r##""#{'\''}""##, r##""#{"'"}""##),
        ] {
            crate::cops::testing::assert_correction(&StringLiteralsInInterpolation, before, after);
        }
    }

    #[test]
    fn flags_single_quotes_in_interpolation_with_message() {
        let found = offenses("\"#{'A'}\"\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 4));
        assert_eq!(
            found[0].message,
            "Prefer double-quoted strings inside interpolations."
        );
    }

    #[test]
    fn flags_rubocop_spec_offenses() {
        for (source, expected) in [
            (":\"#{'A'}\"\n", vec![(1, 5)]),
            ("<<RUBY\n#{'A'}\nRUBY\n", vec![(2, 3)]),
            (
                "/foo#{'sar'.sub('s', 'b')}/\n",
                vec![(1, 7), (1, 17), (1, 22)],
            ),
        ] {
            assert_eq!(positions(source), expected, "{source:?}");
        }
    }

    #[test]
    fn accepts_rubocop_spec_examples() {
        for source in [
            "\"A\"\n",
            "'A'\n",
            "\"A\" \\\n  \"B\"\n",
            "def m\n  puts 'A'\n  puts 'B'\nend\n",
            "if __FILE__ == $PROGRAM_NAME\nend\n",
            "a = ?/\n",
            "\"#{\"A\"}\"\n",
        ] {
            assert!(offenses(source).is_empty(), "should accept {source:?}");
        }
    }

    #[test]
    fn flags_strings_at_any_depth_once() {
        for (source, expected) in [
            ("\"#{success ? 'PASS' : 'FAIL'}\"\n", vec![(1, 14), (1, 23)]),
            ("\"#{\"#{'a'}\"}\"\n", vec![(1, 7)]),
            ("\"#{'a' \"#{'b'}\"}\"\n", vec![(1, 4), (1, 11)]),
            ("\"#{`#{'a'}`}\"\n", vec![(1, 7)]),
            ("if /#{'a'}/\nend\n", vec![(1, 7)]),
            ("%W[#{'a'} b]\n", vec![(1, 6)]),
            ("%I[#{'a'}]\n", vec![(1, 6)]),
            ("/#{`#{'z'}`}/\n", vec![(1, 7)]),
            ("\"#{foo do\n  'a'\nend}\"\n", vec![(2, 3)]),
        ] {
            assert_eq!(positions(source), expected, "{source:?}");
        }
    }

    #[test]
    fn accepts_lookalikes() {
        for source in [
            "`#{'a'}`\n",
            "\"#{'it\"s'}\"\n",
            "\"#{'\\n'}\"\n",
            "\"#{%q(a)}\"\n",
            "\"#{'a\nb'}\"\n",
            "\"#{<<~'EOS'}\"\n  text\nEOS\n",
            "\"#{x}\" 'y'\n",
            "\"#@x 'y'\"\n",
        ] {
            assert!(offenses(source).is_empty(), "should accept {source:?}");
        }
    }

    #[test]
    fn honors_directives() {
        assert!(
            offenses("\"#{'a'}\" # rubocop:disable Style/StringLiteralsInInterpolation\n")
                .is_empty()
        );
    }
}
