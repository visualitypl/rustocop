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
                .then(|| crate::offense::Correction {
                    range: rescue.keyword_loc().end_offset()..assoc.end_offset(),
                    replacement: String::new(),
                }),
        });
    }
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
