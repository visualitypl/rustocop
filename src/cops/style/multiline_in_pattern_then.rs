//! `Style/MultilineInPatternThen` — omit unnecessary multiline pattern delimiters.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MultilineInPatternThen;
impl Cop for MultilineInPatternThen {
    fn name(&self) -> &'static str {
        "Style/MultilineInPatternThen"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_in_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(n) = node.as_in_node() else { return };
        let Some(then) = n.then_loc() else { return };
        let pattern = super::in_pattern_then::pattern(&n);
        if file.line_of(pattern.location().start_offset()) != file.last_line_of(pattern.location())
            || n.statements().is_some_and(|b| {
                file.line_of(n.in_loc().start_offset()) == file.line_of(b.location().start_offset())
            })
            || super::multiline_if_then::ends_with_open_range(&pattern)
        {
            return;
        }
        let (line, col) = file.line_col(then.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Do not use `then` for multiline `in` statement.".into(),
            correction: cx
                .corrections_requested()
                .then(|| {
                    // A guard's open range also consumes the delimiter, but is
                    // separate from the upstream pattern-only detection exemption.
                    let full_pattern = n.pattern();
                    if full_pattern.location().end_offset() != pattern.location().end_offset()
                        && super::multiline_if_then::ends_with_open_range(&full_pattern)
                    {
                        return None;
                    }
                    // A standalone `then` can terminate a guard continued with
                    // `\\` on the previous physical line. Without it a leading
                    // unary body expression can become part of the guard.
                    let gap =
                        &file.bytes[full_pattern.location().end_offset()..then.start_offset()];
                    if gap.windows(2).any(|w| w == b"\\\n")
                        || gap.windows(3).any(|w| w == b"\\\r\n")
                    {
                        return None;
                    }
                    super::multiline_if_then::remove_then(then, false, cx, file)
                })
                .flatten(),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported RuboCop multiline_in_pattern_then_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};

    #[test]
    fn guard_endless_range_keeps_its_grammatical_delimiter_but_reports() {
        for guard in ["if", "unless"] {
            let source = format!("case value\nin x {guard} 1.. then\n work\nend\n");
            let found = correction_offenses(&MultilineInPatternThen, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn multiline_guard_inline_body_and_escaped_newlines_keep_required_then() {
        for source in [
            "case 1\nin x if\n true then -1\nend\n",
            "case 1\nin x unless\n false then -1\nend\n",
            "case 1\nin x if true \\\n then -1\nend\n",
            "case 1\nin x if true \\\r\n then -1\r\nend\r\n",
        ] {
            let found = correction_offenses(&MultilineInPatternThen, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn guards_pins_and_comments_preserve_pattern_binding_and_lines() {
        let source = "\u{feff}# coding: utf-8\r\npin = 1\r\ncase value\r\nin [x, ^pin] if x > 0 then # keep\r\n [x, __LINE__, \"é\"]\r\nin {key: y} unless y.nil?\r\n  then [y, __LINE__]\r\nend\r\n__END__\r\npayload\x00\r\n";
        assert_correction(
            &MultilineInPatternThen,
            source,
            &source.replace(" then #", " #").replace("  then [", " ["),
        );
    }

    #[test]
    fn required_range_and_multiline_pattern_delimiters_are_retained() {
        for source in [
            "case x\nin 1.. then\n work\nend",
            "case x\nin 1... then\nend",
            "case x\nin [a,\n b] then\n work\nend",
            "case x\nin a,\n b then work\nend",
            "case x\nin a then [\n 1\n]\nend",
            "case x\nin a then work\n other\nend",
            "case x\nwhen a then\n work\nend",
            "x in [a]; work",
            // Even a guarded endless pattern gets the upstream pattern exemption.
            "case x\nin 1.. if ready then\n work\nend",
        ] {
            assert!(
                correction_offenses(&MultilineInPatternThen, source).is_empty(),
                "{source}"
            );
        }
        assert_correction(
            &MultilineInPatternThen,
            "case x\nin (1..) then\n work\nend",
            "case x\nin (1..)\n work\nend",
        );
    }

    #[test]
    fn empty_inline_branch_and_data_source_observers_report_without_edits() {
        for source in [
            "case x\nin a then end\n",
            "case x\nin a then\n DATA.pos\nend\n__END__\npayload\n",
            "case x\nin a then # rubocop:disable Lint/Debugger\n work\nend\n",
            "case x\nin a then\n work\nend\np File.read(__FILE__)\n",
        ] {
            let found = correction_offenses(&MultilineInPatternThen, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn whitespace_deletion_is_lazy_disjoint_and_individually_capped() {
        let source = "case value\nin [x, y] then\n work(x, y)\nend\n".repeat(6000);
        let found = crate::pipeline::lint_str_with(&[&MultilineInPatternThen], &source);
        assert_eq!(found.len(), 6000);
        assert!(found.iter().all(|o| o.correction.is_none()));
        let found = correction_offenses(&MultilineInPatternThen, &source);
        assert_eq!(
            found.iter().filter(|o| o.correction.is_some()).count(),
            6000
        );
        assert_eq!(
            crate::corrections::apply(source.as_bytes(), &found)
                .unwrap()
                .bytes,
            source.replace(" then", "").as_bytes()
        );
        let source = format!("case value\nin x{}then\n work\nend", " ".repeat(20000));
        let found = correction_offenses(&MultilineInPatternThen, &source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn all_pinned_diagnostics_and_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/multiline_in_pattern_then_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 14);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/MultilineInPatternThen")
                .collect();
            assert_eq!(
                serde_json::json!(found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>()),
                c["positions"],
                "{c}"
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                c["messages"],
                "{c}"
            );
            if let Some(after) = c["correction"].as_str() {
                assert_correction(&MultilineInPatternThen, source, after);
            }
        }
    }
}
