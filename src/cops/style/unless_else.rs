//! `Style/UnlessElse` — put the positive branch first.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct UnlessElse;
impl Cop for UnlessElse {
    fn name(&self) -> &'static str {
        "Style/UnlessElse"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_unless_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(n) = node.as_unless_node() else {
            return;
        };
        let Some(e) = n.else_clause() else { return };
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            message: "Do not use `unless` with `else`. Rewrite these with the positive case first."
                .into(),
            correction: cx
                .corrections_requested()
                .then(|| correct(&n, &e, cx, file))
                .flatten(),
        });
    }
}
fn correct(
    n: &ruby_prism::UnlessNode<'_>,
    e: &ruby_prism::ElseNode<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let root = cx.ancestors().next_back()?;
    let loc = n.location();
    if loc.end_offset() - loc.start_offset() > 16 * 1024
        || cx.ancestors().any(|a| {
            a.as_unless_node()
                .is_some_and(|u| u.else_clause().is_some())
        })
        || file.contains_heredoc(root, &n.as_node())
        || file.heredoc_pending_at(root, loc.start_offset())
        || loc.as_slice().windows(8).any(|w| w == b"__LINE__")
    {
        return None;
    }
    let mut boundary = n
        .then_keyword_loc()
        .map_or(n.predicate().location().end_offset(), |l| l.end_offset());
    // Prism does not expose a semicolon as then_keyword_loc. Keep it with
    // the header so swapping an inline else body cannot remove the delimiter.
    if n.then_keyword_loc().is_none() {
        let mut at = boundary;
        while at < e.else_keyword_loc().start_offset()
            && matches!(file.bytes[at], b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c')
        {
            at += 1;
        }
        if file.bytes.get(at) == Some(&b';') {
            boundary = at + 1;
        } else {
            let suffix =
                &file.bytes[e.else_keyword_loc().end_offset()..n.end_keyword_loc()?.start_offset()];
            let first = suffix
                .iter()
                .find(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c'));
            if !matches!(first, Some(b'\n' | b';' | b'#')) {
                return None;
            }
        }
    }
    let a = boundary..e.else_keyword_loc().start_offset();
    let b = e.else_keyword_loc().end_offset()..n.end_keyword_loc()?.start_offset();
    if !file.unless_else_bindings(root).swap_safe(a.start..b.end) {
        return None;
    }
    let comments = file.comments();
    let first = comments.partition_point(|r| r.end <= a.start);
    if comments[first..]
        .iter()
        .take_while(|r| r.start < b.end)
        .any(|r| !super::correction::comment_relocation_safe(&file.bytes[r.clone()]))
    {
        return None;
    }
    let edit = super::correction::combine(
        file,
        vec![
            Correction {
                range: n.keyword_loc().start_offset()..n.keyword_loc().end_offset(),
                replacement: "if".into(),
            },
            Correction {
                range: a.clone(),
                replacement: std::str::from_utf8(&file.bytes[b.clone()]).ok()?.into(),
            },
            Correction {
                range: b,
                replacement: std::str::from_utf8(&file.bytes[a]).ok()?.into(),
            },
        ],
    )?;
    super::negated_if::finish(edit, cx, file)
}

/// One lazy VariableForce declaration index plus potential lexical reads.
/// Moving declarations past a same-name read/vcall can change Ruby binding,
/// including captures and modifier assignment. Write-only branches are safe.
#[derive(Default)]
pub(crate) struct Bindings {
    declarations: Vec<(usize, Vec<u8>)>,
    reads: std::collections::HashMap<Vec<u8>, Vec<usize>>,
    blocks: Vec<usize>,
}
impl Bindings {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        #[cfg(test)]
        BUILDS.with(|v| v.set(v.get() + 1));
        let mut result = Self::default();
        if let Some(p) = root.as_program_node() {
            let analysis = crate::cops::variables::ScopeAnalysis::of(&p);
            for scope in analysis.scopes_in_exit_order() {
                for var in analysis.scope(scope).variables() {
                    let var = analysis.variable(var);
                    result
                        .declarations
                        .push((var.declaration_range.start, var.name.to_vec()));
                }
            }
        }
        result.declarations.sort_unstable();
        crate::cops::walk::walk(&mut result, root);
        for reads in result.reads.values_mut() {
            reads.sort_unstable();
        }
        result.blocks.sort_unstable();
        result
    }
    fn swap_safe(&self, range: std::ops::Range<usize>) -> bool {
        let first = self
            .declarations
            .partition_point(|(at, _)| *at < range.start);
        let declarations = &self.declarations[first..];
        if let Some((_, name)) = declarations.first().filter(|(at, _)| *at < range.end) {
            // Preserve lexical local-table order as observed by local_variables
            // and binding, not just each individual read's classification.
            if declarations
                .iter()
                .skip(1)
                .take_while(|(p, _)| *p < range.end)
                .any(|(_, other)| other != name)
            {
                return false;
            }
            let block = self.blocks.partition_point(|&at| at < range.start);
            if self.blocks.get(block).is_some_and(|&at| at < range.end) {
                return false;
            }
        }
        declarations
            .iter()
            .take_while(|(at, _)| *at < range.end)
            .all(|(_, name)| {
                let Some(reads) = self.reads.get(name) else {
                    return true;
                };
                let first = reads.partition_point(|&at| at < range.start);
                reads.get(first).is_none_or(|&at| at >= range.end)
            })
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Bindings {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        VISITS.with(|v| v.set(v.get() + 1));
        if matches!(node, Node::BlockNode { .. } | Node::LambdaNode { .. }) {
            self.blocks.push(node.location().start_offset());
        }
        let name = if let Some(n) = node.as_local_variable_read_node() {
            Some(n.name())
        } else {
            node.as_call_node()
                .filter(|n| n.receiver().is_none())
                .map(|n| n.name())
        };
        if let Some(name) = name {
            self.reads
                .entry(name.as_slice().to_vec())
                .or_default()
                .push(node.location().start_offset());
        }
    }
}
#[cfg(test)]
thread_local! {
    static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    //! RuboCop unless_else_spec.rb examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn all_pinned_diagnostics_and_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/unless_else_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 5);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/UnlessElse")
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
                assert_correction(&UnlessElse, source, after);
            }
        }
    }
    #[test]
    fn unsafe_binding_line_heredoc_directive_and_comment_moves_keep_findings() {
        for source in [
            "unless x\n a\nelse\n a = 1\nend\n",
            "unless x\n a = 1\nelse\n b = 2\nend\np local_variables\n",
            "unless x\n a unless (a = 1)\nelse\n a = 2\nend\n",
            "unless x\n proc { a = 1 }\nelse\n a = 2\nend\n",
            "unless x\n proc { a ||= 1 }\nelse\n a = 2\nend\n",
            "unless x\n p __LINE__\nelse\n p 2\n p 3\nend\n",
            "unless x\n work\nelse\n # shareable_constant_value: literal\n other\nend\n",
            "unless x\n work\nelse\n # rubocop:disable Lint/Debugger\n other\nend\n",
            "unless x\n puts <<~TEXT\n value\n TEXT\nelse\n other\nend\n",
            "unless x\n work\nelse other\nend\n",
        ] {
            let found = correction_offenses(&UnlessElse, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn canonical_then_inline_comments_empty_and_source_envelopes() {
        for (before, after) in [
            ("unless x then a else b end", "if x then b else a end"),
            ("unless x; a; else b; end", "if x; b; else a; end"),
            ("unless x\nelse\n b\nend", "if x\n b\nelse\nend"),
            ("unless x\n a\nelse\nend", "if x\nelse\n a\nend"),
            (
                "\u{feff}# coding: utf-8\r\nunless x # A\r\n \"é\"\r\nelse # B\r\n :b\r\nend\r\np __LINE__\r\n__END__\r\npayload\x00\r\n",
                "\u{feff}# coding: utf-8\r\nif x # B\r\n :b\r\nelse # A\r\n \"é\"\r\nend\r\np __LINE__\r\n__END__\r\npayload\x00\r\n",
            ),
        ] {
            assert_correction(&UnlessElse, before, after);
        }
    }
    #[test]
    fn binding_index_is_lazy_once_and_bounded() {
        BUILDS.with(|v| v.set(0));
        VISITS.with(|v| v.set(0));
        let source = "unless ready\n a = 1\nelse\n a = 2\nend\n".repeat(500);
        crate::pipeline::lint_str(&source);
        assert_eq!(BUILDS.with(|v| v.get()), 0);
        let found = correction_offenses(&UnlessElse, &source);
        assert_eq!(found.len(), 500);
        assert!(found.iter().all(|o| o.correction.is_some()));
        assert_eq!(BUILDS.with(|v| v.get()), 1);
        assert!(VISITS.with(|v| v.get()) < 16 * 500 + 2);
        let large = format!(
            "unless ready\n # {}\n a\nelse\n b\nend\n",
            "long".repeat(5000)
        );
        assert!(
            correction_offenses(&UnlessElse, &large)[0]
                .correction
                .is_none()
        );
    }
    #[test]
    fn lookalikes_and_empty_branches() {
        for s in [
            "x unless ready",
            "unless ready; work; end",
            "if ready; a; else b; end",
            "'unless ready; else; end'",
            "# unless ready\n",
        ] {
            assert!(correction_offenses(&UnlessElse, s).is_empty(), "{s}");
        }
        assert_correction(&UnlessElse, "unless x\nelse\nend", "if x\nelse\nend");
    }
}
