//! `Style/PercentQLiterals` — fixed `lower_case_q`, without candidate reparses.

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct PercentQLiterals;

impl Cop for PercentQLiterals {
    fn name(&self) -> &'static str {
        "Style/PercentQLiterals"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_string_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(string) = node.as_string_node() else {
            return;
        };
        let Some(opening) = string
            .opening_loc()
            .filter(|l| l.as_slice().starts_with(b"%Q"))
        else {
            return;
        };
        let Some(closing) = string.closing_loc() else {
            return;
        };
        // Decode only the candidate's noninterpolating escape rules and compare
        // with the original main-parse payload. No candidate/file reparse and no
        // inference from the mere absence of interpolation. Escapes such as
        // \\t, \\u and escaped interpolation must retain their original semantics.
        let raw = string.content_loc();
        // The parser-gem AST represents empty strings as dstr and splits
        // physical multiline strings into dstr fragments (except a final
        // newline). Upstream only dispatches on_str.
        if raw.as_slice().is_empty()
            || raw.as_slice()[..raw.as_slice().len().saturating_sub(1)].contains(&b'\n')
        {
            return;
        }
        let candidate = lower_q_bytes(raw.as_slice(), opening.as_slice()[2], closing.as_slice()[0]);
        if candidate != string.unescaped() {
            return;
        }
        let start = opening.start_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Do not use `%Q` unless interpolation is needed. Use `%q`.".into(),
            line,
            col,
            end_line: file.line_of(opening.end_offset().saturating_sub(1)),
            correction: cx
                .corrections_requested()
                .then(|| {
                    let root = cx.ancestors().next_back()?;
                    if file.class_method_bindings(root).observes_sources {
                        return None;
                    }
                    Some(Correction {
                        range: start + 1..start + 2,
                        replacement: "q".into(),
                    })
                })
                .flatten(),
        });
    }
}

fn lower_q_bytes(raw: &[u8], open: u8, close: u8) -> Vec<u8> {
    let mut result = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if (raw[i] == b'\\'
            && raw
                .get(i + 1)
                .is_some_and(|&b| b == b'\\' || b == open || b == close))
            || (raw[i] == b'\r' && raw.get(i + 1) == Some(&b'\n'))
        {
            i += 1;
        }
        result.push(raw[i]);
        i += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/percent_q_literals_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};

    #[test]
    fn empty_literals_are_upstream_dstr_not_str_dispatch() {
        for source in ["%Q()", "%Q[]", "%Q!!", "p [%Q(), %s[]]"] {
            assert!(
                lint_str(source)
                    .into_iter()
                    .all(|o| o.cop != "Style/PercentQLiterals"),
                "{source}"
            );
        }
    }

    #[test]
    fn multiline_parser_fragments_match_upstream_str_only_dispatch() {
        for source in ["%Q(a\nb)", "%Q(a\r\nb)", "%Q(\n\n)"] {
            assert!(
                lint_str(source)
                    .into_iter()
                    .all(|o| o.cop != "Style/PercentQLiterals"),
                "{source}"
            );
        }
        assert_correction(&PercentQLiterals, "%Q(a\n)", "%q(a\n)");
    }

    #[test]
    fn pinned_diagnostic_and_correction() {
        let found: Vec<_> = lint_str("# heading\n  %Q(hi)\n")
            .into_iter()
            .filter(|o| o.cop == "Style/PercentQLiterals")
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
        assert_eq!(
            found[0].message,
            "Do not use `%Q` unless interpolation is needed. Use `%q`."
        );
        assert_correction(&PercentQLiterals, "%Q(hi)\n", "%q(hi)\n");
    }

    #[test]
    fn pinned_lookalikes_and_semantically_different_escapes() {
        for source in [
            "'hi'",
            "\"hi\"",
            "%q(hi)",
            r"%q(\t)",
            r"%Q(\t)",
            "%Q(#{1 + 2})",
            "%q(#{1 + 2})",
            r"%Q(\n)",
            r"%Q(\x41)",
            r"%Q(\u{41})",
            r"%Q(\101)",
            r"%Q(\!)",
            r"%Q(\#{x})",
            "%Q(#@foo)",
            "%Q(#$foo)",
            "%w[%Q(hi)]",
        ] {
            assert!(
                lint_str(source)
                    .into_iter()
                    .all(|o| o.cop != "Style/PercentQLiterals"),
                "{source}"
            );
        }
    }

    #[test]
    fn decoded_evidence_handles_delimiter_and_backslash_escapes() {
        for (before, after) in [
            (r"%Q(a\)b)", r"%q(a\)b)"),
            (r"%Q!a\!b!", r"%q!a\!b!"),
            (r"%Q(a\\)", r"%q(a\\)"),
            ("%Q(a\r\n)", "%q(a\r\n)"),
            ("%Q{café}", "%q{café}"),
            ("%Q(a) \"#{x}\"", "%q(a) \"#{x}\""),
        ] {
            assert_correction(&PercentQLiterals, before, after);
        }
    }
}
