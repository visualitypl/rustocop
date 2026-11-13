//! `Style/RescueModifier` — prefer normal begin/rescue syntax.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RescueModifier;
impl Cop for RescueModifier {
    fn name(&self) -> &'static str {
        "Style/RescueModifier"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/MethodCallWithArgsParentheses"]
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_rescue_modifier_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(rescue) = node.as_rescue_modifier_node() else {
            return;
        };
        let loc = rescue.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(loc.end_offset().saturating_sub(1)),
            message: "Avoid using `rescue` in its modifier form.".into(),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, cx, file))
                .flatten(),
        });
    }
}

fn closed_literal(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::NilNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::RationalNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::StringNode { .. }
            | Node::SymbolNode { .. }
    )
}

fn correction(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    let rescue = node.as_rescue_modifier_node()?;
    let operation = rescue.expression();
    let fallback = rescue.rescue_expression();
    // Upstream moves executable expressions onto new lines. That changes
    // exceptions' backtraces even if they are rescued and inspected later.
    // Only closed literal leaves in an otherwise inert top-level program have
    // no such observable execution locations. Never broaden this to calls,
    // constants, interpolations, assignments, blocks or method bodies.
    if !closed_literal(&operation) || !closed_literal(&fallback) {
        return None;
    }
    let mut range = node.location().start_offset()..node.location().end_offset();
    for ancestor in cx.ancestors() {
        match ancestor {
            Node::ProgramNode { .. } => {}
            Node::StatementsNode { .. } if ancestor.as_statements_node()?.body().len() == 1 => {}
            Node::ParenthesesNode { .. } => {
                range = ancestor.location().start_offset()..ancestor.location().end_offset();
            }
            _ => return None,
        }
    }
    let root = cx.ancestors().next_back()?;
    if file.contains_heredoc(root, &operation)
        || file.contains_heredoc(root, &fallback)
        || super::correction::contains_comment(file, range.clone())
        || super::correction::touches_directive(file, range.clone())
    {
        return None;
    }
    let start = file.line_start(file.line_of(range.start));
    let prefix = &file.bytes[start..range.start];
    if !prefix.iter().all(|b| matches!(b, b' ' | b'\t')) {
        return None;
    }
    let indent = std::str::from_utf8(prefix).ok()?;
    let operation = std::str::from_utf8(operation.location().as_slice()).ok()?;
    let fallback = std::str::from_utf8(fallback.location().as_slice()).ok()?;
    let line_end = file.line_start(file.line_of(range.end) + 1);
    let newline = if file.bytes[..line_end].ends_with(b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    Some(crate::offense::Correction {
        range,
        replacement: format!(
            "begin{newline}{indent}  {operation}{newline}{indent}rescue{newline}{indent}  {fallback}{newline}{indent}end"
        ),
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/rescue_modifier_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};

    #[test]
    fn every_applicable_pinned_diagnostic() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/rescue_modifier_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 25);
        let mut held = 0;
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == RescueModifier.name())
                .collect();
            let expected: Vec<Vec<u32>> =
                serde_json::from_value(case["positions"].clone()).unwrap();
            if case["spec_line"] == 404 {
                // This example uses Exclude=['**/**']; the real-config CLI
                // fixture test executes it. lint_str intentionally has no path config.
                assert!(expected.is_empty());
                assert_eq!(found.len(), 1);
                continue;
            }
            assert_eq!(
                found
                    .iter()
                    .map(|o| vec![o.line, o.col])
                    .collect::<Vec<_>>(),
                expected,
                "{source}"
            );
            let messages: Vec<String> = serde_json::from_value(case["messages"].clone()).unwrap();
            assert_eq!(
                found.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
                messages,
                "{source}"
            );
            if case["correction"].is_string() {
                // Dynamic expressions move their backtrace line in upstream's
                // multiline expansion. Detection remains complete.
                assert!(
                    correction_offenses(&RescueModifier, source)
                        .iter()
                        .all(|o| o.correction.is_none()),
                    "{source}"
                );
                held += 1;
            }
        }
        assert_eq!(held, 15);
    }

    #[test]
    fn scope_backtrace_and_source_sensitive_cases_keep_the_offense_without_edits() {
        for source in [
            "raise rescue $!",
            "1 rescue error",
            "::Value rescue nil",
            "def f; 1 rescue nil; end",
            "-> { 1 rescue nil }",
            "proc { 1 rescue nil }",
            "begin; 1 rescue nil; ensure; cleanup; end",
            "x = 1 rescue nil",
            "1 rescue nil; raise",
            "1 rescue nil; p __LINE__",
            "\"#{work}\" rescue nil",
            "1 rescue nil rescue false",
            "1 rescue (2 rescue nil)",
            "(1 rescue # retained\nnil)",
            "<<~X rescue nil\nx\nX\n",
            "1 rescue <<~X\nx\nX\n",
            "__LINE__ rescue nil",
            "DATA.pos; 1 rescue nil\n__END__\npayload",
            "\u{feff}1 rescue nil\n",
        ] {
            let found = correction_offenses(&RescueModifier, source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }

    #[test]
    fn closed_literal_bytes_crlf_and_payload_are_preserved() {
        for (before, after) in [
            (
                "1 rescue nil\r\n__END__\r\ndata",
                "begin\r\n  1\r\nrescue\r\n  nil\r\nend\r\n__END__\r\ndata",
            ),
            (
                "# frozen_string_literal: true\n\"a\\x82\\n\" rescue :fallback\n",
                "# frozen_string_literal: true\nbegin\n  \"a\\x82\\n\"\nrescue\n  :fallback\nend\n",
            ),
            (
                "%q(a\nb) rescue false",
                "begin\n  %q(a\nb)\nrescue\n  false\nend",
            ),
        ] {
            assert_correction(&RescueModifier, before, after);
        }
    }

    #[test]
    fn canonical_expansion_for_closed_standalone_literals() {
        for (before, after) in [
            ("1 rescue nil", "begin\n  1\nrescue\n  nil\nend"),
            (
                "(false rescue true)\n",
                "begin\n  false\nrescue\n  true\nend\n",
            ),
            (
                "  \"ok\" rescue nil\n",
                "  begin\n    \"ok\"\n  rescue\n    nil\n  end\n",
            ),
        ] {
            assert_correction(&RescueModifier, before, after);
        }
    }
}
