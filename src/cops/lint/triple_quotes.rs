//! `Lint/TripleQuotes` — redundant runs of string quotes.
use super::adjacent_strings;
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct TripleQuotes;
impl Cop for TripleQuotes {
    fn name(&self) -> &'static str {
        "Lint/TripleQuotes"
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
        let Some(parts) = adjacent_strings::children(node) else {
            return;
        };
        let mut empty: Vec<_> = parts
            .iter()
            .filter(|n| adjacent_strings::empty(n))
            .collect();
        if empty.is_empty()
            || node
                .location()
                .as_slice()
                .iter()
                .take_while(|b| matches!(b, b'\'' | b'"'))
                .count()
                < 3
        {
            return;
        }
        if empty.len() == parts.len() {
            empty.remove(0);
        }
        // Both expression endpoints matter to compilation events: a leading
        // continuation can move :line/:call, and a trailing empty can own an
        // endless body's :return line. Prove the actual batch's survivors, not
        // just its replacement's newline count. All-empty runs keep part 0.
        let first_survivor = parts
            .iter()
            .find(|part| !adjacent_strings::empty(part))
            .unwrap_or(&parts[0]);
        let last_survivor = parts
            .iter()
            .rfind(|part| !adjacent_strings::empty(part))
            .unwrap_or(&parts[0]);
        let preserves_event_lines = file.line_of(first_survivor.location().start_offset())
            == file.line_of(node.location().start_offset())
            && file.last_line_of(last_survivor.location()) == file.last_line_of(node.location());
        let (line, col) = file.reference_line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(), severity: self.severity(), line, col,
            end_line: file.last_line_of(node.location()),
            message: "Delimiting a string with multiple quotes has no effect, use a single quote instead.".into(),
            correction: if cx.corrections_requested() && preserves_event_lines && adjacent_strings::static_parts(&parts, file) { adjacent_strings::removal(&empty, file) } else { None },
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/triple_quotes_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::pipeline::lint_str;
    #[test]
    fn corrections_preserve_first_surviving_literal_line() {
        for (before, after) in [
            (
                "\"\"\"\" \\\n\"authored\".bytes\n",
                "\"\"\"\" \\\n\"authored\".bytes\n",
            ),
            (
                "\"\"\"\" \\\r\n\"authored\".bytes\r\n",
                "\"\"\"\" \\\r\n\"authored\".bytes\r\n",
            ),
            (
                "\"\"\"\" \\\n'' \\\n\"authored\"\"\"\n",
                "\"\"\"\" \\\n'' \\\n\"authored\"\"\"\n",
            ),
            ("\"\"\"authored\" \\\n\"\"\n", "\"\"\"authored\" \\\n\"\"\n"),
            ("\"\"\"\" \\\n\"\"\n", "\"\"\"\" \\\n\"\"\n"),
            ("\"\"\"line\nvalue\"\"\"\n", "\"line\nvalue\"\n"),
        ] {
            let found = crate::cops::testing::correction_offenses(&super::TripleQuotes, before);
            assert_eq!(found.len(), 1, "{before:?}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(found[0].correction.is_none(), before == after, "{before:?}");
            assert_eq!(
                crate::corrections::apply(before.as_bytes(), &found)
                    .unwrap()
                    .bytes,
                after.as_bytes(),
                "{before:?}"
            );
        }
    }
    #[test]
    fn corrections_preserve_last_surviving_literal_line() {
        for source in [
            "\"''\" \\\n\"\"\n",
            "'\"\"' \\\n''\n",
            "\"''\" \\\n\"\" \\\n\"\"\n",
            "\"\"\"authored\" \\\n''\n",
            "\"\"\"\" \\\n''\n",
        ] {
            for source in [source.to_owned(), source.replace('\n', "\r\n")] {
                let found =
                    crate::cops::testing::correction_offenses(&super::TripleQuotes, &source);
                assert_eq!(found.len(), 1, "{source:?}");
                assert!(found[0].correction.is_none(), "{source:?}");
            }
        }
    }
    #[test]
    fn pinned_offenses_and_lookalikes() {
        for (source, col, end_line) in [
            ("\"\"\"a string\"\"\"", 1, 1),
            ("\"\"\"\n  a string\n\"\"\"", 1, 3),
            ("\"\"\"\"\"\"", 1, 1),
            ("\"\"\"\"\"\n  a string\n\"\"\"\"\"", 1, 3),
            ("str = \"#{'''abc'''}\"", 10, 1),
        ] {
            let got: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Lint/TripleQuotes")
                .collect();
            assert_eq!(got.len(), 1, "{source:?}");
            assert_eq!(
                (got[0].line, got[0].col, got[0].end_line),
                (1, col, end_line)
            );
            assert_eq!(
                got[0].message,
                "Delimiting a string with multiple quotes has no effect, use a single quote instead."
            );
        }
        for source in [
            "\" \" \" \" \" \"",
            "str = \"#{abc}\"",
            "str = <<~STRING\n  a string\n  #{interpolation}\nSTRING\n",
            "'' ''",
            "'a''b''c'",
            "'a''''b'",
        ] {
            assert!(
                lint_str(source)
                    .iter()
                    .all(|o| o.cop != "Lint/TripleQuotes"),
                "{source:?}"
            );
        }
    }
}
