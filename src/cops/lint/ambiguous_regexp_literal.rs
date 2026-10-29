//! `Lint/AmbiguousRegexpLiteral` — flags a regexp literal passed as the
//! first argument of a call without parentheses (`p /pattern/`), which reads
//! like a division.
//!
//! Matches RuboCop's semantics, which are driven by the parser's own
//! diagnostic: every place Prism warns "ambiguous `/`" is reported, at the
//! opening `/`.

use crate::{cops::Cop, offense::Offense, source::SourceFile};

/// The `Lint/AmbiguousRegexpLiteral` cop.
pub struct AmbiguousRegexpLiteral;

/// Prism's warning for a `/` lexed as a regexp opening in argument position.
/// ruby-prism's Rust API exposes no warning ids, so it is matched by its
/// fixed message; the tests fail if an upgrade rewords it.
const AMBIGUOUS_SLASH: &str =
    "ambiguous `/`; wrap regexp in parentheses or add a space after `/` operator";

impl Cop for AmbiguousRegexpLiteral {
    fn name(&self) -> &'static str {
        "Lint/AmbiguousRegexpLiteral"
    }

    fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
        for warning in file.warnings() {
            if warning.message != AMBIGUOUS_SLASH {
                continue;
            }
            let (line, col) = file.line_col(warning.range.start);
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: "Ambiguous regexp literal. Parenthesize the method arguments if it's \
                          surely a regexp literal, or add a whitespace to the right of the `/` \
                          if it should be a division."
                    .to_string(),
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
    //! `spec/rubocop/cop/lint/ambiguous_regexp_literal_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert!(
            parsed.errors().next().is_none(),
            "fixture must parse: {source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/AmbiguousRegexpLiteral")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_single_regexp_argument_with_message() {
        let found = offenses("p /pattern/\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 3));
        assert_eq!(
            found[0].message,
            "Ambiguous regexp literal. Parenthesize the method arguments if it's surely a \
             regexp literal, or add a whitespace to the right of the `/` if it should be a \
             division."
        );
    }

    #[test]
    fn flags_regexp_first_argument_shapes() {
        for source in [
            "p /pattern/, foo\n",
            "p /pattern/.do_something\n",
            "p /pattern/.do_something(42)\n",
            "p /pattern/.do_something.do_something\n",
            "p /pattern/, foo do |arg|\nend\n",
            "p /pat#{tern}/\n",
            "p /pattern/i\n",
        ] {
            assert_eq!(positions(source), vec![(1, 3)], "should flag {source:?}");
        }
    }

    #[test]
    fn flags_regexp_in_nested_structure() {
        let source =
            "class MyTest\n  test '#foo' do\n    assert_match /expected/, actual\n  end\nend\n";
        assert_eq!(positions(source), vec![(3, 18)]);
    }

    #[test]
    fn flags_regexp_argument_of_receiverless_call_in_chain() {
        assert_eq!(
            positions("expect('RuboCop').to(match /Cop/)\n"),
            vec![(1, 28)]
        );
        assert_eq!(
            positions("expect('RuboCop').to match /Robo/\n"),
            vec![(1, 28)]
        );
        assert_eq!(positions("puts line.grep /pattern/\n"), vec![(1, 16)]);
    }

    #[test]
    fn flags_each_nested_call() {
        assert_eq!(
            positions("p /pattern/ do\n  p /pattern/\nend\n"),
            vec![(1, 3), (2, 5)]
        );
    }

    #[test]
    fn flags_match_without_parentheses() {
        assert_eq!(
            positions("assert /some pattern/ =~ some_string\n"),
            vec![(1, 8)]
        );
    }

    #[test]
    fn ignores_unambiguous_regexps_and_divisions() {
        for source in [
            "p(/pattern/)\n",
            "assert(/some pattern/ =~ some_string)\n",
            "assert(/some pattern/) =~ some_string\n",
            "x = 4\nx /2\n",
            "a / b\n",
            "a/b\n",
            "foo / 2\n",
            "p x, /pattern/\n",
            "x = /pattern/\n",
            "foo.bar(/a/, /b/)\n",
            "foo /= 2\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
