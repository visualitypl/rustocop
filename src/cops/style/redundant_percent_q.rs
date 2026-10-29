//! `Style/RedundantPercentQ` — use ordinary quotes when percent-q is redundant.
use super::literal_spelling::{
    character_heads, double_quotes_required, safe_edit, single_quoted_bytes,
};
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct RedundantPercentQ;
impl Cop for RedundantPercentQ {
    fn name(&self) -> &'static str {
        "Style/RedundantPercentQ"
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
        let string = node.as_string_node();
        let (opening, closing, decoded) = if let Some(ref n) = string {
            let (Some(open), Some(close)) = (n.opening_loc(), n.closing_loc()) else {
                return;
            };
            (open, close, Some(n.unescaped()))
        } else if let Some(n) = node.as_interpolated_string_node() {
            let (Some(open), Some(close)) = (n.opening_loc(), n.closing_loc()) else {
                return;
            };
            (open, close, None)
        } else {
            return;
        };
        let prefix = opening.as_slice();
        if !prefix.starts_with(b"%q") && !prefix.starts_with(b"%Q") {
            return;
        }
        let lower = prefix[1] == b'q';
        let source_range = opening.start_offset()..closing.end_offset();
        let source = &file.bytes[source_range.clone()];
        let single = source.contains(&b'\'');
        let double = source.contains(&b'"');
        if single && double {
            return;
        }
        static INTERPOLATION: std::sync::LazyLock<regex::bytes::Regex> =
            std::sync::LazyLock::new(|| regex::bytes::Regex::new(r"#\{.+\}").unwrap());
        let interpolation_like = INTERPOLATION.is_match(source);
        // The reference parser splits physical multiline static strings into
        // dstr fragments, except for a single final newline. Its str-only
        // escape exemption must not suppress those dstr offenses.
        let raw_range = opening.end_offset()..closing.start_offset();
        let raw = &file.bytes[raw_range.clone()];
        let upstream_str = decoded.is_some()
            && !raw.is_empty()
            && !multiple_physical_fragments(character_heads(file, raw_range));
        if lower {
            if single && interpolation_like
                || escaped_non_backslash(character_heads(file, source_range))
            {
                return;
            }
        } else if double
            && (interpolation_like
                || upstream_str && double_quotes_required(character_heads(file, source_range)))
        {
            return;
        }
        let quote = if single || !lower && !double {
            b'"'
        } else {
            b'\''
        };
        let correction = cx
            .corrections_requested()
            .then(|| {
                // Never remove physical-newline delimiters. They contribute to LINE,
                // heredoc scheduling and literal values independently of content.
                if prefix.len() != 3 || matches!(prefix[2], b'\n' | b'\r' | b'\\' | b'\'' | b'"') {
                    return None;
                }
                if quote == b'\'' {
                    if single_quoted_bytes(raw).as_deref() != decoded {
                        return None;
                    }
                    // A quoted candidate cannot reproduce escape-forced encodings.
                    if node.as_string_node().is_some_and(|n| {
                        n.is_forced_utf8_encoding() || n.is_forced_binary_encoding()
                    }) {
                        return None;
                    }
                } else if lower {
                    // q->double is permitted only for literal text: no escape or
                    // interpolation can be activated (including #@ and #$ forms).
                    if raw.contains(&b'\\')
                        || raw.windows(2).any(|w| matches!(w, b"#{" | b"#@" | b"#$"))
                    {
                        return None;
                    }
                }
                // Q->double retains the same escape/interpolation grammar. Detection
                // guarantees no double quote in this branch. Old escaped delimiters
                // are also ordinary escapes in double quotes; no body rewrite occurs.
                let edit = super::correction::combine(
                    file,
                    vec![
                        Correction {
                            range: opening.start_offset()..opening.end_offset(),
                            replacement: char::from(quote).to_string(),
                        },
                        Correction {
                            range: closing.start_offset()..closing.end_offset(),
                            replacement: char::from(quote).to_string(),
                        },
                    ],
                )?;
                safe_edit(file, cx, edit)
            })
            .flatten();
        let (line, col) = file.line_col(opening.start_offset());
        let q = if lower { "%q" } else { "%Q" };
        let extra = if lower {
            ""
        } else {
            ", or for dynamic strings that contain double quotes"
        };
        let message = format!(
            "Use `{q}` only for strings that contain both single quotes and double quotes{extra}."
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.line_of(closing.end_offset().saturating_sub(1)),
            correction,
        });
    }
}

fn multiple_physical_fragments(bytes: impl Iterator<Item = u8>) -> bool {
    let mut bytes = bytes.peekable();
    while let Some(byte) = bytes.next() {
        if byte == b'\\' {
            // A continued physical line remains part of the same reference str.
            if bytes.next() == Some(b'\r') {
                bytes.next_if_eq(&b'\n');
            }
        } else if byte == b'\n' && bytes.peek().is_some() {
            return true;
        }
    }
    false
}

fn escaped_non_backslash(bytes: impl Iterator<Item = u8>) -> bool {
    let mut bytes = bytes.peekable();
    while let Some(byte) = bytes.next() {
        if byte == b'\\' && bytes.peek().is_some_and(|&b| b != b'\n') && bytes.next() != Some(b'\\')
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/redundant_percent_q_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };

    #[test]
    fn pinned_diagnostics() {
        for (source, q, extra) in [
            ("%q(hi)", "%q", ""),
            (
                "%Q(hi)",
                "%Q",
                ", or for dynamic strings that contain double quotes",
            ),
        ] {
            let found: Vec<_> = lint_str(&format!("# heading\n  {source}\n"))
                .into_iter()
                .filter(|o| o.cop == RedundantPercentQ.name())
                .collect();
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                format!(
                    "Use `{q}` only for strings that contain both single quotes and double quotes{extra}."
                )
            );
        }
    }

    #[test]
    fn all_pinned_correction_pairs() {
        for (before, after) in [
            ("%q('hi')\n", "\"'hi'\"\n"),
            ("%q(\"hi\")\n", "'\"hi\"'\n"),
            ("%q(hi)\n", "'hi'\n"),
            (r"%q(\\\\foo\\\\)", r"'\\\\foo\\\\'"),
            (
                "%q(foo bar baz) \\\n  'boogers'\n",
                "'foo bar baz' \\\n  'boogers'\n",
            ),
            ("%q(#{foo})\n", "'#{foo}'\n"),
            ("%Q(hi)\n", "\"hi\"\n"),
            ("%Q(\"hi\")\n", "'\"hi\"'\n"),
            ("%Q(hi#{4})\n", "\"hi#{4}\"\n"),
            (
                "%Q(foo bar baz) \\\n  'boogers'\n",
                "\"foo bar baz\" \\\n  'boogers'\n",
            ),
            (
                "%Q(\n  Quoth the Raven \"Nevermore.\"\n)\n",
                "'\n  Quoth the Raven \"Nevermore.\"\n'\n",
            ),
        ] {
            assert_correction(&RedundantPercentQ, before, after);
        }
    }

    #[test]
    fn pinned_no_offense_examples() {
        for source in [
            "%q('\"hi\"')",
            r"%q(\'foo\')",
            r"%q(\\ \'foo\' \\)",
            "/%q?/",
            "%q(it's #{foo})",
            "%Q('\"hi\"')",
            r#"%Q("\thi")"#,
            r#"%Q("\!thi")"#,
            "%Q(\"hi#{4}\")",
            "/%Q?/",
            "s = <<CODE\n%q('hi') # line 1\n%q(\"hi\")\nCODE\n",
            "\"%q(a)#{b}\"",
            "\"%Q(a)#{b}\"",
            "\"%#{b}%q(a)\"",
            "\"%#{b}%Q(a)\"",
            "%q(foo #{'bar'} baz)",
            "%x(hi)",
            "%w[%q(hi)]",
            "'%q(hi)'",
            "%s(%q(hi))",
        ] {
            assert!(
                correction_offenses(&RedundantPercentQ, source).is_empty(),
                "{source}"
            );
        }
    }

    #[test]
    fn empty_multiline_concatenated_and_nested_interpolation_shapes() {
        for (before, after) in [
            ("%q()", "''"),
            ("%Q()", "\"\""),
            ("%Q(a\r\nb)", "\"a\r\nb\""),
            ("%q(a\nb)", "'a\nb'"),
            ("%Q(#{%Q(hi)})", "\"#{%Q(hi)}\""),
        ] {
            // Nested offenses overlap: the outer edit applies atomically first.
            assert_correction(&RedundantPercentQ, before, after);
        }
    }

    #[test]
    fn upstream_multiline_fragment_dispatch_does_not_hide_unsafe_offenses() {
        for source in ["%Q(\n\"hi\\n\"\n)", "%Q(\"a\nb\\n\")"] {
            let found = correction_offenses(&RedundantPercentQ, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none());
        }
        // A single final newline is still a single str in the reference AST.
        for source in [
            "%Q(\"hi\\n\"\n)",
            r#"%Q\"hi"\"#,
            "%Q\"a\\\nb\"",
            "%Q\"a\\\r\nb\"",
        ] {
            assert!(
                correction_offenses(&RedundantPercentQ, source).is_empty(),
                "{source}"
            );
        }
    }

    #[test]
    fn dangerous_switches_remain_reported_without_edits() {
        for source in [
            r#"%Q(\"hi\")"#,
            "%q(it's #@name)",
            "%q(it's #$name)",
            "%q(it's #{})",
            "%Q\nhello\n",
            "%Q\r\nhello\r\n",
            "%q'hi'",
            "%Q\"hi\"",
            r"%Q\hi\",
            r##"%Q("#@name")"##,
            r##"%Q("#$name")"##,
        ] {
            let found = correction_offenses(&RedundantPercentQ, source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert!(found[0].correction.is_none(), "{source:?}");
        }
    }
}
