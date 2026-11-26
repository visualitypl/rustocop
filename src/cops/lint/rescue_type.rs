//! `Lint/RescueType` — literal exception filters that cannot match exceptions.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RescueType;
impl Cop for RescueType {
    fn name(&self) -> &'static str {
        "Lint/RescueType"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_rescue_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(rescue) = node.as_rescue_node() else {
            return;
        };
        let exceptions: Vec<_> = rescue.exceptions().iter().collect();
        let invalid: Vec<_> = exceptions
            .iter()
            .filter(|n| invalid_type(n, file))
            .collect();
        if invalid.is_empty() {
            return;
        }
        let names = invalid
            .iter()
            .map(|n| {
                let raw = file.match_pattern_source(n.location().as_slice());
                raw.utf8_chunks()
                    .map(|c| {
                        let mut s = c.valid().to_owned();
                        for b in c.invalid() {
                            s.push_str(&format!("\\x{b:02X}"));
                        }
                        s
                    })
                    .collect::<String>()
                    .replace("\r\n", "\n")
            })
            .collect::<Vec<_>>()
            .join(", ");
        let (line, col) = file.reference_line_col(rescue.keyword_loc().start_offset());
        out.push(Offense {
            cop: self.name(), severity: self.severity(), line, col,
            end_line: file.line_of(exceptions.last().unwrap().location().end_offset().saturating_sub(1)),
            message: format!("Rescuing from `{names}` will raise a `TypeError` instead of catching the actual exception."),
            correction: cx.corrections_requested().then(|| correction(&rescue, &exceptions, file)).flatten(),
        });
    }
}
/// RuboCop replaces what follows `rescue`, up to the last exception, with
/// the valid exceptions.
fn correction(
    rescue: &ruby_prism::RescueNode<'_>,
    exceptions: &[Node<'_>],
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    let valid = exceptions
        .iter()
        .filter(|n| !invalid_type(n, file))
        .map(|n| std::str::from_utf8(n.location().as_slice()).ok())
        .collect::<Option<Vec<_>>>()?;
    Some(crate::offense::Correction {
        range: rescue.keyword_loc().end_offset()
            ..exceptions.last().unwrap().location().end_offset(),
        replacement: if valid.is_empty() {
            String::new()
        } else {
            format!(" {}", valid.join(", "))
        },
    })
}
fn invalid_type(node: &Node<'_>, file: &SourceFile) -> bool {
    matches!(
        node,
        Node::ArrayNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::InterpolatedStringNode { .. }
            | Node::FalseNode { .. }
            | Node::FloatNode { .. }
            | Node::HashNode { .. }
            | Node::NilNode { .. }
            | Node::IntegerNode { .. }
            | Node::RationalNode { .. }
            | Node::StringNode { .. }
            | Node::TrueNode { .. }
            | Node::SourceLineNode { .. }
            | Node::SourceFileNode { .. }
    ) || crate::cops::ast::is_reference_symbol(node, file)
        || crate::cops::ast::negated_numeric_literal(node).is_some()
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/rescue_type_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;

    #[test]
    fn keeps_the_valid_exceptions() {
        for (before, after) in [
            (
                "begin\n  foo\nrescue nil\n  bar\nend\n",
                "begin\n  foo\nrescue\n  bar\nend\n",
            ),
            (
                "begin\n  foo\nrescue nil, StandardError\n  bar\nend\n",
                "begin\n  foo\nrescue StandardError\n  bar\nend\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&RescueType, before, after);
        }
    }
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/RescueType")
            .collect()
    }
    #[test]
    fn pinned_invalid_taxonomy_all_shared_contexts() {
        for literal in [
            "nil",
            "'string'",
            "\"#{string}\"",
            "0",
            "0.0",
            "[]",
            "{}",
            ":symbol",
            "true",
            "false",
            "1r",
            "1i",
            "-1",
            "-1.0",
            "-1r",
            "-1i",
            "--1",
            "__LINE__",
            "__FILE__",
        ] {
            for (header, ensure) in [
                ("begin", ""),
                ("begin", "ensure\n  baz\n"),
                ("def foobar", ""),
                ("def foobar", "ensure\n  baz\n"),
            ] {
                for types in [
                    literal.to_string(),
                    format!("{literal}, StandardError"),
                    format!("StandardError, {literal}"),
                ] {
                    let source = format!("{header}\n  foo\nrescue {types}\n  bar\n{ensure}end\n");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 3));
                    assert_eq!(
                        found[0].message,
                        format!(
                            "Rescuing from `{literal}` will raise a `TypeError` instead of catching the actual exception."
                        )
                    );
                }
            }
        }
    }
    #[test]
    fn pinned_valid_and_dynamic_lookalikes() {
        for s in [
            "foo rescue nil",
            "begin; foo; rescue; bar; end",
            "def foobar; foo; rescue NameError; bar; end",
            "def foobar; foo; rescue; bar; end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for expr in [
            "NameError",
            "::NameError",
            "Other::Error",
            "error",
            "self.error",
            "*errors",
            ":\"#{name}\"",
            "/error/",
            "%r(#{x})",
            "`never executed`",
            "proc { nil }",
            "(nil)",
            "---1",
            ":''",
            "__ENCODING__",
        ] {
            let s = format!("begin; foo; rescue {expr}; bar; end");
            assert!(offenses(&s).is_empty(), "{s}");
        }
    }
}
