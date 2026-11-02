//! `Lint/NestedPercentLiteral` — apparent percent literals in percent-array words.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct NestedPercentLiteral;
impl Cop for NestedPercentLiteral {
    fn name(&self) -> &'static str {
        "Lint/NestedPercentLiteral"
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
        // PercentLiteral#process is on_array only: %, q/Q/r/s/x are valid
        // inner prefixes but cannot be outer arrays. Bracket arrays never own
        // their child percent literals.
        if open.len() != 3 || ![b"%w".as_slice(), b"%W", b"%i", b"%I"].contains(&&open[..2]) {
            return;
        }
        let single = matches!(open[1], b'w' | b'i');
        let nested = array.elements().iter().any(|child| {
            let parts = if let Some(s) = child.as_string_node() {
                crate::cops::ast::require_literal::parts(
                    s.content_loc().as_slice(),
                    s.unescaped(),
                    single,
                    file,
                )
            } else if let Some(s) = child.as_symbol_node() {
                crate::cops::ast::require_literal::parts(
                    s.value_loc().map_or(s.unescaped(), |l| l.as_slice()),
                    s.unescaped(),
                    single,
                    file,
                )
            } else {
                // dstr/dsym's first child is an AST node, whose #to_s begins
                // with '(' rather than '%', even if its first str looks nested.
                return false;
            };
            // Percent words keep escaped physical whitespace in one scalar
            // str/sym child; ordinary quoted strings' fragment boundaries do
            // not turn these into dstr/dsym nodes.
            nested_prefix(&parts.concat())
        });
        if !nested {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(), severity: self.severity(),
            message: "Within percent literals, nested percent literals do not function and may be unwanted in the result.".into(),
            line, col, end_line: file.last_line_of(node.location()), correction: None,
        });
    }
}
// PreferredDelimiters::PERCENT_LITERAL_TYPES and /\A<prefix>\W/.
// Ruby's unqualified \w is ASCII, not Unicode alphanumeric. Scrubbing invalid
// bytes cannot change these ASCII prefix/word tests.
fn nested_prefix(value: &[u8]) -> bool {
    if value.first() != Some(&b'%') {
        return false;
    }
    let nonword = |b: u8| !b.is_ascii_alphanumeric() && b != b'_';
    value.get(1).is_some_and(|&b| {
        nonword(b) || b"iIqQrswWx".contains(&b) && value.get(2).is_some_and(|&b| nonword(b))
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/nested_percent_literal_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/NestedPercentLiteral")
            .collect()
    }
    #[test]
    fn pinned_offenses() {
        for source in [
            "%i[a b %i[c d] xyz]",
            "%i[a b %i[c d] %i[xyz]]",
            "%i[a %i[b %i[c d]] xyz]",
            r"%W[\xff %W[]]",
        ] {
            let found = offenses(&format!("nil\n  {source}\n"));
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                "Within percent literals, nested percent literals do not function and may be unwanted in the result."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn decoded_static_words_not_dynamic_first_child_source() {
        for source in [
            r"%W[\x25i[a]]",
            r"%I[%i\u0021]",
            "%w[a %i[b\\\nc] d]",
            "%W[a %i[b\\\nc] d]",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "%W[%i[a#{value}]]",
            "%I[%i[a#{value}]]",
            "%W[#{'%i[a]'}]",
            "['%i[a]']",
            "%q[%i[a]]",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_no_offenses() {
        for source in [
            "%i[]",
            "%i[a b c d xyz]",
            "%i[% %i %I %q %Q %r %s %w %W %x]",
            "%[a b %[c d] xyz]",
            "%[a b %i[c d] xyz]",
            "%w[1%+ 2]",
            r"%W[\xff]",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
