//! `Lint/PercentSymbolArray` — suspicious colon/comma-bearing percent symbols.
//! Approved fixed-policy no-edit exception: removing payload changes values.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct PercentSymbolArray;
impl Cop for PercentSymbolArray {
    fn name(&self) -> &'static str {
        "Lint/PercentSymbolArray"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_array_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(array) = node.as_array_node() else {
            return;
        };
        let Some(opening) = array.opening_loc() else {
            return;
        };
        let open = opening.as_slice();
        if open.len() != 3 || !matches!(&open[..2], b"%i" | b"%I") {
            return;
        }
        if !array.elements().iter().any(|child| {
            // A dsym's first child #to_s is an S-expression, never a symbol
            // payload beginning with ':' or ending with ','.
            let Some(s) = child.as_symbol_node() else {
                return false;
            };
            let value = crate::cops::ast::require_literal::parts(
                s.value_loc().map_or(s.unescaped(), |l| l.as_slice()),
                s.unescaped(),
                open[1] == b'i',
                file,
            )
            .concat();
            super::percent_string_array::has_alphanumeric(&value, file)
                && (value.first() == Some(&b':') || value.last() == Some(&b','))
        }) {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: concat!(
                "Within `%i`/`%I`, ':' and ',' are unnecessary and may be ",
                "unwanted in the resulting symbols."
            )
            .into(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/percent_symbol_array_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/PercentSymbolArray")
            .collect()
    }
    #[test]
    fn pinned_offenses() {
        for kind in ["i", "I"] {
            for words in [":foo, :bar, :baz", ":foo bar baz", "foo, bar baz"] {
                let source = format!("nil\n  %{kind}({words})\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                assert_eq!(
                    found[0].message,
                    "Within `%i`/`%I`, ':' and ',' are unnecessary and may be unwanted in the resulting symbols."
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn decoded_absolute_boundaries_and_per_symbol_alphanumeric_gate() {
        for source in [r"%I[\x3aa]", r"%I[a\x2c]", r"%I[:é]", r"%I[:a\nb]"] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [r"%I[a,\nb]", r"%I[a\n:b]", "%i[foo : ,]", r"%I[:\u{1c89}]"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_no_offenses_and_non_array_lookalikes() {
        for kind in ["i", "I"] {
            for words in ["foo bar baz", "$,", ": ,", "#{a} b"] {
                assert!(offenses(&format!("%{kind}({words})")).is_empty());
            }
        }
        for source in ["[:foo, :bar]", "%w(:foo)", "%q(:foo)", "%I[:a#{x},]"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
