//! `Lint/UnifiedInteger` — obsolete unqualified/root integer class aliases.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct UnifiedInteger;
impl Cop for UnifiedInteger {
    fn name(&self) -> &'static str {
        "Lint/UnifiedInteger"
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
        let name = if let Some(read) = node.as_constant_read_node() {
            read.name()
        } else if let Some(path) = node.as_constant_path_node() {
            if path.parent().is_some() || assignment_target(node, cx) {
                return;
            }
            let Some(name) = path.name() else {
                return;
            };
            name
        } else {
            return;
        };
        if !matches!(name.as_slice(), b"Fixnum" | b"Bignum") {
            return;
        }
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(loc.end_offset() - 1),
            message: format!(
                "Use `Integer` instead of `{}`.",
                String::from_utf8_lossy(name.as_slice())
            ),
            correction: cx
                .corrections_requested()
                .then(|| closed_consumed_alias(node, cx, name.as_slice()))
                .flatten()
                .filter(|_| file.data_start().is_none()),
        });
    }
}

/// A closed two-statement program establishes the actual alias before consuming
/// it in a one-element array. Ruby 3.4 removed both native aliases: bare reads
/// raise NameError (even `defined?` changes nil to "constant"). Never repair those
/// errors or assume that a syntactic name identifies Integer. This bounded proof
/// preserves the alias binding itself, and replaces only the consumed read's
/// NAME, retaining any root prefix. No dispatch, namespace body, callbacks,
/// mutation, delayed execution or other observers are admitted. Main-parse shared
/// source/code-object/heap guards still apply. No root rescan or candidate parse.
fn closed_consumed_alias(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    name: &[u8],
) -> Option<crate::offense::Correction> {
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
        || write.value().location().as_slice() != b"::Integer"
        || !write.value().as_constant_path_node().is_some_and(|p| {
            p.parent().is_none() && p.name().is_some_and(|n| n.as_slice() == b"Integer")
        })
    {
        return None;
    }
    let loc = node.location();
    Some(crate::offense::Correction {
        range: loc.end_offset() - name.len()..loc.end_offset(),
        replacement: "Integer".into(),
    })
}

// Scoped write targets are const nodes in Prism but casgn in RuboCop.
fn assignment_target(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    cx.parent().is_some_and(|p| {
        let target = match p {
            Node::ConstantPathWriteNode { .. } => p.as_constant_path_write_node().unwrap().target(),
            Node::ConstantPathOrWriteNode { .. } => {
                p.as_constant_path_or_write_node().unwrap().target()
            }
            Node::ConstantPathAndWriteNode { .. } => {
                p.as_constant_path_and_write_node().unwrap().target()
            }
            Node::ConstantPathOperatorWriteNode { .. } => {
                p.as_constant_path_operator_write_node().unwrap().target()
            }
            _ => return false,
        };
        target.location().start_offset() == node.location().start_offset()
            && target.location().end_offset() == node.location().end_offset()
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/unified_integer_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UnifiedInteger")
            .collect()
    }
    #[test]
    fn complete_pinned_specs_sources_controls_and_real_corrections() {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/integer_uri.json")).unwrap();
        assert_eq!(rows.len(), 1137);
        for row in rows {
            let cop: &dyn crate::cops::Cop = match row["cop"].as_str().unwrap() {
                "Lint/UnifiedInteger" => &super::UnifiedInteger,
                "Lint/UriEscapeUnescape" => &crate::cops::lint::UriEscapeUnescape,
                _ => unreachable!(),
            };
            let source = row["source"].as_str().unwrap();
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            let mut found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == cop.name())
                .map(|o| (o.line, o.col, o.end_line, o.message))
                .collect();
            found.sort();
            assert_eq!(found, expected, "{row}");
            let findings = crate::cops::testing::correction_offenses(cop, source);
            assert_eq!(
                crate::corrections::apply(source.as_bytes(), &findings)
                    .unwrap()
                    .bytes,
                row["project_after"].as_str().unwrap().as_bytes(),
                "{row}"
            );
        }
    }
    #[test]
    fn both_aliases_and_root_qualifiers_have_exact_diagnostics() {
        for name in ["Fixnum", "Bignum"] {
            for root in ["", "::"] {
                let source = format!("# before\n1.is_a?({root}{name})\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 9, 2));
                assert_eq!(
                    found[0].message,
                    format!("Use `Integer` instead of `{name}`.")
                );
            }
        }
    }
    #[test]
    fn closed_consumed_alias_has_a_canonical_name_only_correction() {
        for name in ["Fixnum", "Bignum"] {
            for root in ["", "::"] {
                let before = format!("{name} = ::Integer\n[{root}{name}]\n");
                let after = format!("{name} = ::Integer\n[{root}Integer]\n");
                let found =
                    crate::cops::testing::correction_offenses(&super::UnifiedInteger, &before);
                assert_eq!(found.len(), 1);
                assert!(found[0].correction.is_some(), "{before}");
                assert_eq!(
                    crate::corrections::apply(before.as_bytes(), &found)
                        .unwrap()
                        .bytes,
                    after.as_bytes()
                );
            }
        }
    }
    #[test]
    fn other_names_namespaces_and_assignment_targets_are_clean() {
        for source in [
            "1.is_a?(Integer)",
            "1.is_a?(::Integer)",
            "1.is_a?(MyNamespace::Integer)",
            "MyNamespace::Fixnum",
            "::Other::Bignum",
            "Fixnum = 1",
            "::Bignum = 1",
            "::Fixnum ||= 1",
            "::Bignum, other = values",
            "# Fixnum\n'Bignum'\n:Fixnum",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
