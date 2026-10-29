//! `Lint/ConstantOverwrittenInRescue` — implicit rescue binding to a constant.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct ConstantOverwrittenInRescue;
impl Cop for ConstantOverwrittenInRescue {
    fn name(&self) -> &'static str {
        "Lint/ConstantOverwrittenInRescue"
    }
    fn safe_fixes(&self) -> bool {
        true
    }

    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &[
            "Naming/RescuedExceptionsVariableName",
            "Style/RescueStandardError",
        ]
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
        if !rescue.exceptions().is_empty() || rescue.statements().is_some() {
            return;
        }
        let Some(target) = rescue.reference().filter(|target| {
            matches!(
                target,
                Node::ConstantTargetNode { .. } | Node::ConstantPathTargetNode { .. }
            )
        }) else {
            return;
        };
        let Some(assoc) = rescue.operator_loc() else {
            return;
        };
        let (line, col) = file.line_col(assoc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(assoc.end_offset() - 1),
            message: format!(
                "`{}` is overwritten by `rescue =>`.",
                String::from_utf8_lossy(target.location().as_slice())
            ),
            correction: cx
                .corrections_requested()
                .then(|| {
                    inert_correction(
                        cx,
                        file,
                        rescue.keyword_loc().end_offset(),
                        assoc.end_offset(),
                    )
                })
                .flatten(),
        });
    }
}

/// Unlike the upstream intention-repair rewrite, changing a reachable handler
/// changes both its exception filter and its constant write. Only a first rescue
/// whose protected body has no evaluation/lookup/allocation can be rewritten.
/// Do not infer purity of calls, constants, containers, or arbitrary literals.
/// The shared run_cops source/position/code-object/heap barriers still apply.
fn inert_correction(
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    start: usize,
    end: usize,
) -> Option<crate::offense::Correction> {
    // Later rescue clauses are nested under RescueNode, not BeginNode. They
    // may be reached through a preceding filter and are deliberately withheld.
    let begin = cx.parent()?.as_begin_node()?;
    if begin.statements().is_some_and(|statements| {
        statements.body().iter().any(|node| {
            !matches!(
                node,
                Node::NilNode { .. } | Node::TrueNode { .. } | Node::FalseNode { .. }
            )
        })
    }) {
        return None;
    }
    // Trace/eval hooks can raise even at a literal's line event. Reuse the
    // existing lazy, typed-child-complete observer proof, not a per-node scan.
    let root = cx.ancestors().next_back()?;
    if file.class_method_bindings(root).observes_sources() {
        return None;
    }
    let removed = &file.bytes[start..end];
    // Exact canonical keyword.end..assoc.end deletion, never comments or physical
    // newlines (including escaped newlines). The trailing space is untouched.
    let whitespace = removed.strip_suffix(b"=>")?;
    whitespace
        .iter()
        .all(|byte| matches!(byte, b' ' | b'\t'))
        .then(|| crate::offense::Correction {
            range: start..end,
            replacement: String::new(),
        })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/constant_overwritten_in_rescue_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;

    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == ConstantOverwrittenInRescue.name())
            .collect()
    }

    #[test]
    fn every_pinned_spec_source_and_control_matches_exact_diagnostics_and_guarded_edits() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/constant_overwritten_in_rescue.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 151);
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
                crate::cops::testing::correction_offenses(&ConstantOverwrittenInRescue, source);
            let applied = crate::corrections::apply(source.as_bytes(), &corrections).unwrap();
            assert_eq!(
                applied.bytes,
                row["project_after"].as_str().unwrap().as_bytes(),
                "{row}"
            );
        }
    }

    #[test]
    fn incompatible_selected_peers_withhold_only_edits_not_detection() {
        let before = "begin; nil; rescue => Error; end";
        let cop = ConstantOverwrittenInRescue;
        for peer in [
            &crate::cops::naming::RescuedExceptionsVariableName as &dyn Cop,
            &crate::cops::style::RescueStandardError as &dyn Cop,
        ] {
            for active in [vec![&cop as &dyn Cop, peer], vec![peer, &cop as &dyn Cop]] {
                let found = crate::pipeline::lint_str_with_corrections(&active, before);
                let offense = found.iter().find(|o| o.cop == cop.name()).unwrap();
                assert!(offense.correction.is_none(), "{}", peer.name());
            }
        }
        assert!(
            crate::cops::testing::correction_offenses(&cop, before)[0]
                .correction
                .is_some()
        );
    }

    #[test]
    fn source_code_heap_and_runtime_observers_withhold_individual_edits() {
        for observer in [
            "File.binread(__FILE__)",
            "IO.read(__FILE__)",
            "RubyVM::InstructionSequence.compile_file(__FILE__)",
            "RubyVM::AbstractSyntaxTree.parse_file(__FILE__)",
            "ObjectSpace.each_object {}",
            "GC.stat",
            "TracePoint.new(:line) {}.enable",
            "set_trace_func(proc {})",
            "eval('nil')",
            "DATA.pos",
        ] {
            let source =
                format!("begin; nil; rescue => Error; end\n{observer}\n__END__\npayload\n");
            let found =
                crate::cops::testing::correction_offenses(&ConstantOverwrittenInRescue, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn comments_line_values_bom_crlf_and_unobserved_data_are_preserved() {
        for before in [
            "\u{feff}begin\r\n  nil # protected\r\nrescue => Error # binding\r\n # empty\r\nend\r\np __LINE__\r\n__END__\r\npayload",
            "# coding: UTF-8\nbegin; true; rescue\t=> Érror; end\n",
            "text = <<~TEXT\n  keep\nTEXT\nbegin; nil; rescue => Error; end\n",
        ] {
            let after = before
                .replace("rescue =>", "rescue")
                .replace("rescue\t=>", "rescue");
            crate::cops::testing::assert_correction(&ConstantOverwrittenInRescue, before, &after);
        }
    }

    #[test]
    fn pinned_constant_bindings_report_the_assoc_and_exact_constant_source() {
        for target in [
            "StandardError",
            "::StandardError",
            "foo.class::RESCUABLE_EXCEPTIONS",
            "var::StandardError",
            "MyNamespace::MyException",
            "::MyNamespace::MyException",
        ] {
            let source = format!("var = Object\nbegin\n  something\nrescue => {target}\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (4, 8, 4));
            assert_eq!(
                found[0].message,
                format!("`{target}` is overwritten by `rescue =>`.")
            );
            assert_eq!(found[0].severity, crate::offense::Severity::Error);
        }
    }

    #[test]
    fn canonical_fix_only_for_inert_protected_bodies() {
        for body in ["", "nil", "true", "false"] {
            for target in [
                "StandardError",
                "::StandardError",
                "Missing::Error",
                "factory::Error",
            ] {
                let before = format!("begin; {body}; rescue => {target}; end");
                let after = before.replace("rescue =>", "rescue");
                crate::cops::testing::assert_correction(
                    &ConstantOverwrittenInRescue,
                    &before,
                    &after,
                );
            }
        }
    }

    #[test]
    fn reachable_or_uncertain_handlers_keep_diagnostics_without_corrections() {
        for source in [
            "begin; something; rescue => StandardError; end",
            "begin; raise 'error'; rescue => StandardError; end",
            "begin; 1; rescue => StandardError; end",
            "begin; []; rescue => StandardError; end",
            "begin; \"#{work}\"; rescue => StandardError; end",
            "begin; nil; rescue Error; work; rescue => StandardError; end",
            "begin; nil; rescue \\\n => StandardError; end",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&ConstantOverwrittenInRescue, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn pinned_no_offense_and_exact_empty_body_implicit_filter_lookalikes() {
        for source in [
            "begin; something; rescue StandardError; end",
            "begin; something; rescue StandardError => e; end",
            "begin; something; rescue => e; end",
            "begin; something; rescue StandardError => Other; end",
            "begin; something; rescue *errors => Other; end",
            "begin; something; rescue => Other; nil; end",
            "begin; something; rescue => Other; handler; end",
            "something rescue Other",
            "object.rescue",
            "# rescue => Other\n'rescue => Other'",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
