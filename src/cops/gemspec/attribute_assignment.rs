//! `Gemspec/AttributeAssignment`: consistent direct/indexed style per attribute
//! across the whole source. Detection only; combining assignments changes order,
//! evaluation and values and is not an upstream canonical correction.
use crate::{
    cops::{Cop, ast},
    offense::Offense,
    source::SourceFile,
};
use std::collections::HashSet;
pub struct AttributeAssignment;
impl Cop for AttributeAssignment {
    fn name(&self) -> &'static str {
        "Gemspec/AttributeAssignment"
    }
    fn check_parse(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let assignments = super::help::assignments(&result.node(), file).nodes;
        let direct: HashSet<_> = assignments
            .iter()
            .filter(|a| a.index.is_none())
            .map(|a| &a.attribute[..a.attribute.len() - 1])
            .collect();
        for a in &assignments {
            if a.index.is_some() && direct.contains(a.attribute.as_slice()) {
                let start = a.node.location().start_offset();
                let (line, col) = crate::cops::lint::reference_line_col(start, file);
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    line,
                    col,
                    end_line: file.line_of(ast::source_end(&a.node).saturating_sub(1).max(start)),
                    message: "Use consistent style for Gemspec attributes assignment.".into(),
                    correction: None,
                });
            }
        }
    }
}
#[cfg(test)]
mod tests {
    //! RuboCop attribute_assignment_spec.rb, MIT (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{
        config::ResolvedConfig,
        cops::{Registry, select_active},
    };
    #[test]
    fn reference_buffer_columns_compose_mac_and_count_first_line_bom() {
        for (source, line, col) in [
            (
                "# coding: UTF8-MAC\nGem::Specification.new { |s| cafe\u{301} = 0; s.metadata = {}; s.metadata[:x] = 2 }\n",
                2,
                57,
            ),
            (
                "\u{feff}Gem::Specification.new { |s| s.metadata = {}; s.metadata[:x] = 2 }\n",
                1,
                48,
            ),
        ] {
            let config = ResolvedConfig::default();
            let registry = Registry::new();
            let selected = select_active(
                &registry,
                &config,
                &[AttributeAssignment.name().into()],
                &[],
            );
            let report = crate::pipeline::lint_one(
                std::path::Path::new("a.gemspec"),
                std::path::Path::new("a.gemspec"),
                Some(source.as_bytes()),
                &selected,
                &config,
            );
            assert!(report.errors.is_empty(), "{report:?}");
            assert!(report.internal_errors.is_empty(), "{report:?}");
            assert_eq!(report.result.offenses.len(), 1, "{source}: {report:?}");
            let o = &report.result.offenses[0];
            assert_eq!((o.line, o.col, o.end_line), (line, col, line), "{source}");
            assert_eq!(
                o.message,
                "Use consistent style for Gemspec attributes assignment."
            );
            assert!(o.correction.is_none());
        }
    }
    #[test]
    fn whole_source_consistency_and_positions() {
        let config = ResolvedConfig::default();
        let registry = Registry::new();
        let selected = select_active(
            &registry,
            &config,
            &[AttributeAssignment.name().into()],
            &[],
        );
        let r = crate::pipeline::lint_one(std::path::Path::new("a.gemspec"), std::path::Path::new("a.gemspec"), Some(b"Gem::Specification.new do |spec|\n  spec.metadata = {}\n  spec.metadata['key'] = 1\nend\n"), &selected, &config);
        assert_eq!(r.result.offenses.len(), 1);
        let o = &r.result.offenses[0];
        assert_eq!((o.line, o.col, o.end_line), (3, 3, 3));
        assert_eq!(
            o.message,
            "Use consistent style for Gemspec attributes assignment."
        );
    }
}
