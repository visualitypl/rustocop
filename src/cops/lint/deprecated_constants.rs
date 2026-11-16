//! `Lint/DeprecatedConstants` — exact fixed Ruby 3.4 deprecated constant reads.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct DeprecatedConstants;
impl Cop for DeprecatedConstants {
    fn name(&self) -> &'static str {
        "Lint/DeprecatedConstants"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        matches!(
            node,
            Node::ConstantReadNode { .. } | Node::ConstantPathNode { .. }
        )
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if !matches!(
            node,
            Node::ConstantReadNode { .. } | Node::ConstantPathNode { .. }
        ) {
            return;
        }
        // Prism represents a scoped assignment target as a ConstantPathNode;
        // RuboCop's corresponding node is casgn, not a const callback.
        if cx.parent().is_some_and(|parent| {
            let target = match parent {
                Node::ConstantPathWriteNode { .. } => {
                    parent.as_constant_path_write_node().unwrap().target()
                }
                Node::ConstantPathOrWriteNode { .. } => {
                    parent.as_constant_path_or_write_node().unwrap().target()
                }
                Node::ConstantPathAndWriteNode { .. } => {
                    parent.as_constant_path_and_write_node().unwrap().target()
                }
                Node::ConstantPathOperatorWriteNode { .. } => parent
                    .as_constant_path_operator_write_node()
                    .unwrap()
                    .target(),
                _ => return false,
            };
            target.location().start_offset() == node.location().start_offset()
                && target.location().end_offset() == node.location().end_offset()
        }) {
            return;
        }
        let source = node.location().as_slice();
        // Upstream uses node.source.delete_prefix('::'), NOT const-name
        // normalization. Whitespace, newlines and parentheses are significant.
        let constant = source.strip_prefix(b"::").unwrap_or(source);
        let (good, version) = match constant {
            b"NIL" => ("nil", "2.4"),
            b"TRUE" => ("true", "2.4"),
            b"FALSE" => ("false", "2.4"),
            b"Net::HTTPServerException" => ("Net::HTTPClientException", "2.6"),
            b"Random::DEFAULT" => ("Random.new", "3.0"),
            b"Struct::Group" => ("Etc::Group", "3.0"),
            b"Struct::Passwd" => ("Etc::Passwd", "3.0"),
            _ => return,
        };
        // Every fixed entry is deprecated by Ruby 3.4. No public table/version knobs.
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(node.location().end_offset() - 1),
            message: format!(
                "Use `{good}` instead of `{}`, deprecated since Ruby {version}.",
                String::from_utf8_lossy(source)
            ),
            // RuboCop replaces the constant with its alternative.
            correction: cx
                .corrections_requested()
                .then(|| crate::offense::Correction {
                    range: node.location().start_offset()..node.location().end_offset(),
                    replacement: good.into(),
                }),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/deprecated_constants_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;

    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == DeprecatedConstants.name())
            .collect()
    }

    #[test]
    fn all_seven_fixed_entries_and_root_qualifiers_have_exact_diagnostics() {
        for (bad, good, version) in [
            ("NIL", "nil", "2.4"),
            ("TRUE", "true", "2.4"),
            ("FALSE", "false", "2.4"),
            (
                "Net::HTTPServerException",
                "Net::HTTPClientException",
                "2.6",
            ),
            ("Random::DEFAULT", "Random.new", "3.0"),
            ("Struct::Group", "Etc::Group", "3.0"),
            ("Struct::Passwd", "Etc::Passwd", "3.0"),
        ] {
            for prefix in ["", "::"] {
                let source = format!("# before\n  {prefix}{bad}\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                assert_eq!(
                    found[0].message,
                    format!(
                        "Use `{good}` instead of `{prefix}{bad}`, deprecated since Ruby {version}."
                    )
                );
            }
        }
    }

    #[test]
    fn closed_literal_alias_is_a_real_canonical_correction() {
        for (name, value) in [("NIL", "nil"), ("TRUE", "true"), ("FALSE", "false")] {
            for root in ["", "::"] {
                let before =
                    format!("# alias\n{name} = {value} # binding\n[{root}{name}] # read\n");
                let after = format!("# alias\n{name} = {value} # binding\n[{value}] # read\n");
                let found =
                    crate::cops::testing::correction_offenses(&DeprecatedConstants, &before);
                assert_eq!(found.len(), 1);
                assert!(found[0].correction.is_some(), "{before}");
                assert_eq!(
                    crate::corrections::apply(before.as_bytes(), &found)
                        .unwrap()
                        .bytes,
                    after.as_bytes()
                );
                assert!(offenses(&after).is_empty());
            }
        }
    }

    #[test]
    fn exact_source_not_normalized_paths_or_runtime_resolution() {
        for source in [
            "Foo::TRUE",
            "::Foo::NIL",
            "nil; true; false",
            "Net::HTTPClientException",
            "Random.new",
            "Etc::Group; Etc::Passwd",
            "__ENCODING__",
            "Triple::Nested::Constant",
            "Have::No::Alternative",
            "Have::No::DeprecatedVersion",
            "Net ::HTTPServerException",
            "Net:: HTTPServerException",
            ":: Net::HTTPServerException",
            "Random::\n DEFAULT",
            "(Random)::DEFAULT",
            "Struct :: Group",
            "NIL = nil",
            "::TRUE = true",
            "Net::HTTPServerException = 1",
            "# NIL\n'TRUE'\n:FALSE\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        // It is deliberately syntactic: namespaces/shadowing don't remove findings.
        for source in [
            "module M; NIL; end",
            "NIL = 1; NIL",
            "class Random; DEFAULT; end",
        ] {
            assert_eq!(
                offenses(source).len(),
                usize::from(!source.contains("DEFAULT")),
                "{source}"
            );
        }
    }
}
