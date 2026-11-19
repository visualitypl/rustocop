//! `Lint/PercentStringArray` — suspicious quotes/commas in percent words.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct PercentStringArray;
impl Cop for PercentStringArray {
    fn name(&self) -> &'static str {
        "Lint/PercentStringArray"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_array_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
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
        if open.len() != 3 || !matches!(&open[..2], b"%w" | b"%W") {
            return;
        }
        if !array.elements().iter().any(|child| {
            let Some(s) = child.as_string_node() else {
                // A dstr's first child is itself an AST node. Its #to_s is
                // an S-expression, not the interpolated value or first str.
                return false;
            };
            let value = crate::cops::ast::require_literal::parts(
                s.content_loc().as_slice(),
                s.unescaped(),
                open[1] == b'w',
                file,
            )
            .concat();
            has_alphanumeric(&value, file)
                && value.split(|&b| b == b'\n').any(|line| {
                    line.last() == Some(&b',')
                        || line.len() >= 2
                            && matches!(line[0], b'\'' | b'"')
                            && line.last() == line.first()
                })
        }) {
            return;
        }
        // Each element loses a leading quote and a trailing quote and comma.
        let correction = cx
            .corrections_requested()
            .then(|| {
                let mut edits = Vec::new();
                for element in array.elements().iter() {
                    let loc = element.location();
                    let source = loc.as_slice();
                    let lead = usize::from(matches!(source.first(), Some(b'\'' | b'"')));
                    let mut trail = usize::from(source.last() == Some(&b','));
                    if matches!(
                        source.get(source.len().wrapping_sub(trail + 1)),
                        Some(b'\'' | b'"')
                    ) && source.len() > trail
                    {
                        trail += 1;
                    }
                    let (start, end) = (loc.start_offset(), loc.end_offset());
                    if lead + trail >= source.len() {
                        edits.push((start..end, String::new()));
                        continue;
                    }
                    edits.push((start..start + lead, String::new()));
                    edits.push((end - trail..end, String::new()));
                }
                crate::offense::Correction::from_edits(&file.bytes, edits)
            })
            .flatten();
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: concat!(
                "Within `%w`/`%W`, quotes and ',' are unnecessary and may be ",
                "unwanted in the resulting strings."
            )
            .into(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction,
        });
    }
}

// Literal payloads are already projected to Parser::Source::Buffer's UTF-8,
// except binary sources. Invalid escape result bytes are scrubbed upstream;
// neither scrubbing nor invalid bytes can introduce an alphanumeric character.
pub(super) fn has_alphanumeric(value: &[u8], file: &SourceFile) -> bool {
    if file.match_pattern_binary() {
        value.iter().any(u8::is_ascii_alphanumeric)
    } else {
        value.utf8_chunks().any(|chunk| {
            chunk
                .valid()
                .chars()
                .any(crate::source::projected_alphanumeric)
        })
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/percent_string_array_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/PercentStringArray")
            .collect()
    }
    #[test]
    fn pinned_offenses() {
        for kind in ["w", "W"] {
            for words in [
                "'foo', 'bar', 'baz'",
                "'foo' \"bar\" 'baz'",
                "'foo' bar baz",
                "foo, bar baz",
            ] {
                let source = format!("nil\n  %{kind}({words})\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                assert_eq!(
                    found[0].message,
                    "Within `%w`/`%W`, quotes and ',' are unnecessary and may be unwanted in the resulting strings."
                );
                assert!(found[0].correction.is_none());
            }
        }
        assert_eq!(offenses(r#"%W("a\255\255")"#).len(), 1);
    }
    #[test]
    fn decoded_line_anchors_and_per_word_alphanumeric_gate() {
        // Ruby's ^/$ are line anchors, not absolute string anchors. Alnum
        // must occur in the SAME word, but need not occur on the matched line.
        for source in [
            r"%W[a,\nb]",
            r"%W['a'\nb]",
            r"%W[a\n'b']",
            r"%W[a\n,]",
            r"%W['é']",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [r#"%W("a\nb")"#, r#"%w[foo , '"]"#, r"%W['\u{1c89}']"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_no_offenses_and_non_array_lookalikes() {
        for kind in ["w", "W"] {
            for words in ["foo bar baz", "' \"", "' \" ! = # ,", "#{a} b"] {
                assert!(offenses(&format!("%{kind}({words})")).is_empty());
            }
        }
        for source in [
            r##":"#{a}""##,
            r"%W(\255)",
            "['foo,']",
            "%q('foo')",
            "%i(:foo)",
            "%W['foo#{x}']",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
