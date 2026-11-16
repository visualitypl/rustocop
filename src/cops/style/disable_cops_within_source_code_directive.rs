//! `Style/DisableCopsWithinSourceCodeDirective`: opt-in ban on source directives.
use super::correction::combine;
use crate::{
    cops::{Cop, NodeContext},
    directives::comments::DirectiveComment,
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct DisableCopsWithinSourceCodeDirective;
impl Cop for DisableCopsWithinSourceCodeDirective {
    fn name(&self) -> &'static str {
        "Style/DisableCopsWithinSourceCodeDirective"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_none() {
            return;
        }
        let records = file.directive_records();
        let mut corrected_until = 0;
        for d in records {
            if !d.has_cops() {
                continue;
            }
            let (line, col) = file.line_col(d.range.start);
            // Atomic push/pop spans overlap nested directives. The existing
            // first-wins engine would defer those inner edits anyway; avoid
            // copying their overlapping interiors (quadratic on deep stacks).
            let correction = (cx.corrections_requested() && d.range.start >= corrected_until)
                .then(|| removal(file, d, records))
                .flatten();
            if let Some(correction) = &correction {
                corrected_until = correction.range.end;
            }
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "RuboCop disable/enable directives are not permitted.".into(),
                line,
                col,
                end_line: file.line_of(d.range.end.saturating_sub(1).max(d.range.start)),
                correction,
            });
        }
    }
}
/// RuboCop's `remove_directive`: the comment, and a push's matching pop.
fn removal(
    file: &SourceFile,
    d: &DirectiveComment,
    records: &[DirectiveComment],
) -> Option<Correction> {
    let mut edits = Vec::new();
    for record in std::iter::once(d).chain(d.pop.map(|pop| &records[pop])) {
        // Prism includes CR in an inline comment's location, but it belongs
        // to the physical CRLF terminator, not the comment being removed.
        let mut range = record.range.clone();
        if file.bytes[range.clone()].last() == Some(&b'\r')
            && file.bytes.get(range.end) == Some(&b'\n')
        {
            range.end -= 1;
        }
        edits.push(Correction {
            range,
            replacement: String::new(),
        });
    }
    combine(file, edits)
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/disable_cops_within_source_code_directive_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn comments_without_captured_cops_and_literal_mentions_are_ignored() {
        for source in [
            "# rubocop:disable\n",
            "# rustocop:push\nfoo\n# rustocop:pop\n",
            "# mention rubocop:disable Lint/Debugger\n",
            "## rubocop:disable Lint/Debugger\n",
            "# # rustocop:enable all\n",
            "#\u{b}# rustocop:enable all\n",
            "# rubocop:disabled all\n",
            "# rustocop:disableé Lint/Debugger\n",
            "# rubocop:todo\u{301} all\n",
            "# rubocop:push Metrics/AbcSize\n",
            "# rubocop:next Metrics/AbcSize\n",
            "x = '# rubocop:disable Lint/Debugger'\n",
            "x = <<~H\n# rubocop:disable all\nH\n",
            "__END__\n# rubocop:disable all\n",
        ] {
            assert!(
                correction_offenses(&DisableCopsWithinSourceCodeDirective, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn embedded_directives_are_real_but_plain_mentions_are_not() {
        let source = "# example # rubocop:disable Migration/Nonexistent -- legacy\n";
        let found = correction_offenses(&DisableCopsWithinSourceCodeDirective, source);
        assert_eq!((found[0].line, found[0].col), (1, 1));
        assert_correction(&DisableCopsWithinSourceCodeDirective, source, "\n");
        assert_correction(
            &DisableCopsWithinSourceCodeDirective,
            "#\u{b}rustocop\u{b}:\u{b}todo\u{b}Migration/Nonexistent\n",
            "\n",
        );
    }
    #[test]
    fn nested_atomic_corrections_copy_only_disjoint_source_spans() {
        let source = format!(
            "{}work\n{}",
            "# rubocop:push -Metrics/AbcSize\n".repeat(1000),
            "# rubocop:pop\n".repeat(1000)
        );
        let found = correction_offenses(&DisableCopsWithinSourceCodeDirective, &source);
        assert_eq!(found.len(), 1000);
        let copied: usize = found
            .iter()
            .filter_map(|o| o.correction.as_ref())
            .map(|c| c.range.len())
            .sum();
        assert!(copied <= source.len(), "{copied} > {}", source.len());
    }
}
