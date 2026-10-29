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
            correction: cx
                .corrections_requested()
                .then(|| closed_literal_alias(node, cx, constant, good))
                .flatten()
                // DATA is itself an observable IO binding with a byte position.
                // Use the existing main-parse metadata, never inspect its payload.
                .filter(|_| file.data_start().is_none()),
        });
    }
}

/// A standalone closed program: exactly a top-level literal alias write and
/// a single-element array consuming its read. Bare void reads are withheld:
/// Ruby -w distinguishes a useless constant read from a useless nil/boolean.
/// Ordinary NIL/TRUE/FALSE are removed in Ruby 3.4; rewriting an unresolved
/// read changes NameError. Random::DEFAULT is also absent at 3.4
/// (and was a class, not a fresh instance, in Ruby 3.0). Never infer runtime
/// equivalence from the deprecated name.
///
/// The two direct statements exclude calls/callbacks, namespace changes,
/// reassignments, private/deprecated/autoload constants, exception handlers,
/// delayed evaluation and observers. The same array is allocated with the same
/// singleton element on both sides, without user dispatch. This is a constant-time
/// parent/sibling proof, not a per-node root scan or global type flow. Shared source/position/
/// code-object/heap guards still filter the resulting edit in run_cops.
fn closed_literal_alias(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    name: &[u8],
    good: &str,
) -> Option<crate::offense::Correction> {
    if !matches!(name, b"NIL" | b"TRUE" | b"FALSE") {
        return None;
    }
    let mut ancestors = cx.ancestors();
    let array = ancestors.next()?.as_array_node()?;
    if array.elements().len() != 1 {
        return None;
    }
    let statements = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some() || statements.body().len() != 2 {
        return None;
    }
    let body = statements.body();
    let mut nodes = body.iter();
    let write = nodes.next()?.as_constant_write_node()?;
    let read = nodes.next()?;
    if read.location().start_offset() != array.location().start_offset()
        || read.location().end_offset() != array.location().end_offset()
        || write.name().as_slice() != name
        || write.value().location().as_slice() != good.as_bytes()
        || !matches!(
            write.value(),
            Node::NilNode { .. } | Node::TrueNode { .. } | Node::FalseNode { .. }
        )
    {
        return None;
    }
    Some(crate::offense::Correction {
        range: node.location().start_offset()..node.location().end_offset(),
        replacement: good.into(),
    })
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
    fn complete_pinned_specs_documentation_and_source_controls() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/deprecated_constants.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 279);
        for row in rows {
            let source = row["source"].as_str().unwrap();
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            let found = offenses(source);
            assert_eq!(
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.end_line, o.message.clone()))
                    .collect::<Vec<_>>(),
                expected,
                "{row}"
            );
            let corrections =
                crate::cops::testing::correction_offenses(&DeprecatedConstants, source);
            assert_eq!(
                crate::corrections::apply(source.as_bytes(), &corrections)
                    .unwrap()
                    .bytes,
                row["project_after"].as_str().unwrap().as_bytes(),
                "{row}"
            );
        }
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
    fn uncertain_reads_keep_findings_without_edits() {
        for source in [
            "NIL",
            "TRUE",
            "FALSE",
            "Net::HTTPServerException",
            "Random::DEFAULT",
            "Struct::Group",
            "Struct::Passwd",
            "NIL = nil; ::NIL", // -w distinguishes a void constant from nil.
            "TRUE = true; TRUE",
            "FALSE = false; ::FALSE",
            "NIL = 1; ::NIL",
            "NIL = true; ::NIL",
            "NIL = nil; TRUE",
            "::NIL = nil; ::NIL",
            "module M; NIL = nil; NIL; end",
            "NIL = nil; module M; NIL; end",
            "NIL = nil; task { ::NIL }",
            "NIL = nil; -> { ::NIL }",
            "NIL = nil; defined?(::NIL)",
            "NIL = nil; p ::NIL",
            "NIL = nil; (::NIL)",
            "NIL = nil; ::NIL; NIL = 1",
            "NIL = nil; NIL = 1; ::NIL",
            "NIL ||= nil; ::NIL",
            "NIL = nil; private_constant :NIL; ::NIL",
            "NIL = nil; deprecate_constant :NIL; ::NIL",
            "autoload :NIL, 'alias'; NIL",
            "def Object.const_added(x); raise; end; NIL = nil; ::NIL",
            "NIL = nil; Object.send(:remove_const, :NIL); ::NIL",
            "NIL = nil; eval('NIL=1'); ::NIL",
            "TracePoint.new(:line) {}.enable; NIL = nil; ::NIL",
            "NIL = nil; ::NIL; File.read(__FILE__)",
            "NIL = nil; ::NIL; RubyVM::InstructionSequence.of(method(:foo))",
            "NIL = nil; ::NIL; ObjectSpace.each_object {}",
            "NIL = nil; ::NIL; p __LINE__",
            "NIL = nil; ::NIL; p DATA.read\n__END__\ndata",
            "NIL = nil; \"#{::NIL}\"",
            "NIL = nil; <<~DOC\n#{::NIL}\nDOC\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&DeprecatedConstants, source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
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
